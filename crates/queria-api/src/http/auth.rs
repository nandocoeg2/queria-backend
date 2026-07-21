use crate::app::ApiState;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use queria_core::auth::password::PasswordHasher;
use queria_core::auth::session::SessionIssuer;
use queria_db::repositories::{
    AuthenticatedSession, PgAuthRepository, resolve_active_organization_id,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    authenticated: bool,
    user_id: Option<String>,
    email: Option<String>,
    expires_at: Option<String>,
    active_organization_id: Option<String>,
    active_organization_slug: Option<String>,
    is_platform_super_admin: Option<bool>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct MeResponse {
    authenticated: bool,
    user_id: Option<String>,
    email: Option<String>,
    active_organization_id: Option<String>,
    active_organization_slug: Option<String>,
    is_platform_super_admin: Option<bool>,
    error: Option<String>,
}

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/login", post(login))
        .route("/me", get(me))
}

async fn login(
    State(state): State<ApiState>,
    Json(payload): Json<LoginRequest>,
) -> (StatusCode, HeaderMap, Json<LoginResponse>) {
    let mut headers = HeaderMap::new();
    let Some(repository) = state.auth_repository() else {
        return login_error(
            StatusCode::UNAUTHORIZED,
            headers,
            "user_store_not_configured",
        );
    };

    let email = payload.email.trim().to_lowercase();
    if email.is_empty() || payload.password.is_empty() {
        return login_error(StatusCode::BAD_REQUEST, headers, "invalid_login_payload");
    }

    let Ok(Some(user)) = repository.find_user_by_email(&email).await else {
        return login_error(StatusCode::UNAUTHORIZED, headers, "invalid_credentials");
    };

    let password_valid = PasswordHasher
        .verify_password(&payload.password, &user.password_hash)
        .unwrap_or(false);
    if !password_valid {
        return login_error(StatusCode::UNAUTHORIZED, headers, "invalid_credentials");
    }

    let active_organization_id =
        resolve_active_organization_id(user.membership_organization_id, user.organization_id);
    let is_platform_super_admin = effective_platform_super_admin(
        user.is_platform_super_admin,
        &user.email,
        &state.config.platform_super_admin_emails,
    );

    let issued = SessionIssuer.issue_session_token();
    let expires_at = Utc::now() + Duration::days(7);
    if repository
        .create_session(
            user.id,
            &issued.token_prefix,
            &issued.token_hash,
            expires_at,
            active_organization_id,
        )
        .await
        .is_err()
    {
        return login_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            headers,
            "session_create_failed",
        );
    }

    let cookie = format!(
        "queria_session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800",
        issued.raw_token
    );
    if let Ok(value) = cookie.parse() {
        headers.insert(header::SET_COOKIE, value);
    }

    let active_organization_slug = if active_organization_id.is_some() {
        user.membership_organization_slug
    } else {
        None
    };

    (
        StatusCode::OK,
        headers,
        Json(LoginResponse {
            authenticated: true,
            user_id: Some(user.id.to_string()),
            email: Some(user.email),
            expires_at: Some(expires_at.to_rfc3339()),
            active_organization_id: active_organization_id.map(|id| id.to_string()),
            active_organization_slug,
            is_platform_super_admin: Some(is_platform_super_admin),
            error: None,
        }),
    )
}

async fn me(State(state): State<ApiState>, headers: HeaderMap) -> (StatusCode, Json<MeResponse>) {
    let session = match require_session(&state, &headers).await {
        Ok(session) => session,
        Err(error) => return me_error(error),
    };

    (
        StatusCode::OK,
        Json(MeResponse {
            authenticated: true,
            user_id: Some(session.user_id.to_string()),
            email: Some(session.email),
            active_organization_id: session.active_organization_id.map(|id| id.to_string()),
            active_organization_slug: session.active_organization_slug,
            is_platform_super_admin: Some(session.is_platform_super_admin),
            error: None,
        }),
    )
}

pub async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, &'static str> {
    let Some(repository) = state.auth_repository() else {
        return Err("user_store_not_configured");
    };

    let Some(raw_token) = session_cookie(headers) else {
        return Err("session_required");
    };

    let token_hash = SessionIssuer::hash_session_token(raw_token);
    let Ok(Some(mut session)) = repository.find_session_by_hash(&token_hash).await else {
        return Err("invalid_session");
    };

    // Apply env bootstrap on top of DB flag (case-insensitive email list).
    session.is_platform_super_admin = effective_platform_super_admin(
        session.is_platform_super_admin,
        &session.email,
        &state.config.platform_super_admin_emails,
    );

    Ok(session)
}

/// Tenant routes: principal must have a bound active organization.
/// Maps to HTTP 403 (not empty 200).
pub fn require_active_org(session: &AuthenticatedSession) -> Result<Uuid, &'static str> {
    session
        .active_organization_id
        .ok_or("active_organization_required")
}

/// Platform org management routes: principal must be platform super-admin.
/// Maps to HTTP 403.
pub fn require_platform_super_admin(session: &AuthenticatedSession) -> Result<(), &'static str> {
    if session.is_platform_super_admin {
        Ok(())
    } else {
        Err("platform_super_admin_required")
    }
}

/// DB flag OR case-insensitive membership in env email list.
#[must_use]
pub fn effective_platform_super_admin(
    db_flag: bool,
    email: &str,
    configured_emails: &[String],
) -> bool {
    db_flag || PgAuthRepository::email_in_platform_super_admin_list(email, configured_emails)
}

fn login_error(
    status: StatusCode,
    headers: HeaderMap,
    message: &str,
) -> (StatusCode, HeaderMap, Json<LoginResponse>) {
    (
        status,
        headers,
        Json(LoginResponse {
            authenticated: false,
            user_id: None,
            email: None,
            expires_at: None,
            active_organization_id: None,
            active_organization_slug: None,
            is_platform_super_admin: None,
            error: Some(message.to_owned()),
        }),
    )
}

fn me_error(message: &str) -> (StatusCode, Json<MeResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(MeResponse {
            authenticated: false,
            user_id: None,
            email: None,
            active_organization_id: None,
            active_organization_slug: None,
            is_platform_super_admin: None,
            error: Some(message.to_owned()),
        }),
    )
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie_header| {
            cookie_header.split(';').find_map(|part| {
                part.trim()
                    .strip_prefix("queria_session=")
                    .filter(|token| !token.is_empty())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ApiState, build_app_with_pool};
    use ::http::{Request, StatusCode as HttpStatus};
    use axum::body::{Body, to_bytes};
    use chrono::Utc;
    use queria_core::AppConfig;
    use queria_core::auth::password::PasswordHasher;
    use queria_core::auth::session::SessionIssuer;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn sample_session(
        active_organization_id: Option<Uuid>,
        is_platform_super_admin: bool,
    ) -> AuthenticatedSession {
        AuthenticatedSession {
            user_id: Uuid::now_v7(),
            email: "ops@example.com".to_owned(),
            expires_at: Utc::now() + Duration::hours(1),
            active_organization_id,
            active_organization_slug: active_organization_id.map(|_| "home-org".to_owned()),
            is_platform_super_admin,
        }
    }

    #[test]
    fn require_active_org_forbids_null_home() {
        let session = sample_session(None, true);
        let err = require_active_org(&session).expect_err("null home must 403");
        assert_eq!(err, "active_organization_required");
    }

    #[test]
    fn require_active_org_ok_with_home() {
        let org = Uuid::now_v7();
        let session = sample_session(Some(org), false);
        assert_eq!(require_active_org(&session).expect("home present"), org);
    }

    #[test]
    fn require_platform_super_admin_forbids_ordinary_user() {
        let session = sample_session(Some(Uuid::now_v7()), false);
        let err = require_platform_super_admin(&session).expect_err("ordinary member");
        assert_eq!(err, "platform_super_admin_required");
    }

    #[test]
    fn require_platform_super_admin_ok_for_flag() {
        let session = sample_session(None, true);
        require_platform_super_admin(&session).expect("super-admin allowed");
    }

    #[test]
    fn env_bootstrap_elevates_super_admin_without_db_flag() {
        let emails = vec!["Admin@Example.com".to_owned()];
        assert!(effective_platform_super_admin(
            false,
            "admin@example.com",
            &emails
        ));
        assert!(!effective_platform_super_admin(
            false,
            "other@example.com",
            &emails
        ));
        assert!(effective_platform_super_admin(true, "x@y.z", &[]));
    }

    #[test]
    fn super_admin_gate_does_not_satisfy_active_org() {
        let session = sample_session(None, true);
        require_platform_super_admin(&session).expect("platform ok");
        assert!(require_active_org(&session).is_err());
    }

    async fn test_pool() -> Option<PgPool> {
        let url = std::env::var("QUERIA_DATABASE_URL").ok()?;
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .ok()
    }

    async fn seed_user(
        pool: &PgPool,
        email: &str,
        password: &str,
        with_membership: bool,
        is_platform_super_admin: bool,
    ) -> (Uuid, Uuid) {
        let slug = format!("org-{}", Uuid::now_v7().simple());
        let org_id: Uuid =
            sqlx::query_scalar("insert into organization(slug, name) values ($1, $2) returning id")
                .bind(&slug)
                .bind(format!("Org {slug}"))
                .fetch_one(pool)
                .await
                .expect("insert org");

        let password_hash = PasswordHasher
            .hash_password(password)
            .expect("hash password");
        let user_id: Uuid = sqlx::query_scalar(
            "insert into user_account(
               organization_id, email, password_hash, role, is_platform_super_admin
             )
             values ($1, $2, $3, 'admin', $4)
             returning id",
        )
        .bind(org_id)
        .bind(email)
        .bind(password_hash)
        .bind(is_platform_super_admin)
        .fetch_one(pool)
        .await
        .expect("insert user");

        if with_membership {
            sqlx::query(
                "insert into org_membership(user_id, organization_id, role)
                 values ($1, $2, 'org_admin')",
            )
            .bind(user_id)
            .bind(org_id)
            .execute(pool)
            .await
            .expect("insert membership");
        }

        (user_id, org_id)
    }

    async fn cleanup_user(pool: &PgPool, user_id: Uuid, org_id: Uuid) {
        let _ = sqlx::query("delete from user_session where user_id = $1")
            .bind(user_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from user_account where id = $1")
            .bind(user_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from organization where id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
    }

    #[tokio::test]
    async fn login_binds_active_organization_for_member() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let email = format!("member-{}@session.test", Uuid::now_v7().simple());
        let password = "correct horse battery staple";
        let (user_id, org_id) = seed_user(&pool, &email, password, true, false).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"email":"{email}","password":"{password}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), HttpStatus::OK);

        let body = to_bytes(response.into_body(), 1024 * 64)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            json["active_organization_id"].as_str(),
            Some(org_id.to_string().as_str())
        );
        assert_eq!(json["is_platform_super_admin"], false);

        let stored: Option<Uuid> = sqlx::query_scalar(
            "select active_organization_id from user_session
             where user_id = $1
             order by created_at desc
             limit 1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session row");
        assert_eq!(stored, Some(org_id));

        cleanup_user(&pool, user_id, org_id).await;
    }

    #[tokio::test]
    async fn login_super_admin_without_membership_has_null_active_org() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let email = format!("super-{}@session.test", Uuid::now_v7().simple());
        let password = "correct horse battery staple";
        let (user_id, org_id) = seed_user(&pool, &email, password, false, true).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"email":"{email}","password":"{password}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), HttpStatus::OK);
        let body = to_bytes(response.into_body(), 1024 * 64)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(json["active_organization_id"].is_null());
        assert_eq!(json["is_platform_super_admin"], true);

        let stored: Option<Uuid> = sqlx::query_scalar(
            "select active_organization_id from user_session
             where user_id = $1
             order by created_at desc
             limit 1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session");
        assert!(stored.is_none());

        cleanup_user(&pool, user_id, org_id).await;
    }

    #[tokio::test]
    async fn auth_me_exposes_active_org_and_super_admin() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let email = format!("me-{}@session.test", Uuid::now_v7().simple());
        let password = "correct horse battery staple";
        let (user_id, org_id) = seed_user(&pool, &email, password, true, false).await;

        let mut config = AppConfig::default_local();
        config.platform_super_admin_emails = vec![email.clone()];

        let app = build_app_with_pool(config, pool.clone());
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"email":"{email}","password":"{password}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("login");
        assert_eq!(login.status(), HttpStatus::OK);
        let cookie = login
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("set-cookie")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_owned();

        let me = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/me")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("me request"),
            )
            .await
            .expect("me");
        assert_eq!(me.status(), HttpStatus::OK);
        let body = to_bytes(me.into_body(), 1024 * 64).await.expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            json["active_organization_id"].as_str(),
            Some(org_id.to_string().as_str())
        );
        assert!(
            json["active_organization_slug"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "me must expose active_organization_slug for members: {json}"
        );
        assert_eq!(json["is_platform_super_admin"], true);

        cleanup_user(&pool, user_id, org_id).await;
    }

    #[tokio::test]
    async fn failed_login_does_not_mint_session() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let email = format!("fail-{}@session.test", Uuid::now_v7().simple());
        let password = "correct horse battery staple";
        let (user_id, org_id) = seed_user(&pool, &email, password, true, false).await;

        let before: i64 =
            sqlx::query_scalar("select count(*) from user_session where user_id = $1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .expect("count");

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"email":"{email}","password":"wrong password here"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), HttpStatus::UNAUTHORIZED);
        assert!(response.headers().get(header::SET_COOKIE).is_none());

        let after: i64 = sqlx::query_scalar("select count(*) from user_session where user_id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(before, after);

        cleanup_user(&pool, user_id, org_id).await;
    }

    #[tokio::test]
    async fn projects_forbidden_without_active_org() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let email = format!("nohome-{}@session.test", Uuid::now_v7().simple());
        let password = "correct horse battery staple";
        let (user_id, org_id) = seed_user(&pool, &email, password, false, true).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"email":"{email}","password":"{password}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("login");
        assert_eq!(login.status(), HttpStatus::OK);
        let cookie = login
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("cookie")
            .split(';')
            .next()
            .expect("pair")
            .to_owned();

        let projects = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/projects")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("projects"),
            )
            .await
            .expect("response");
        assert_eq!(projects.status(), HttpStatus::FORBIDDEN);

        cleanup_user(&pool, user_id, org_id).await;
    }

    #[tokio::test]
    async fn membership_org_preferred_over_diverging_legacy() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let email = format!("divergent-{}@session.test", Uuid::now_v7().simple());
        let password = "correct horse battery staple";
        let (user_id, legacy_org) = seed_user(&pool, &email, password, true, false).await;

        let other_slug = format!("other-{}", Uuid::now_v7().simple());
        let other_org: Uuid =
            sqlx::query_scalar("insert into organization(slug, name) values ($1, $2) returning id")
                .bind(&other_slug)
                .bind("Other")
                .fetch_one(&pool)
                .await
                .expect("other org");

        // Pathological: move membership to other_org while leaving legacy column.
        sqlx::query("delete from org_membership where user_id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .expect("clear membership");
        sqlx::query(
            "insert into org_membership(user_id, organization_id, role)
             values ($1, $2, 'org_admin')",
        )
        .bind(user_id)
        .bind(other_org)
        .execute(&pool)
        .await
        .expect("new membership");

        let repo = PgAuthRepository::new(pool.clone());
        let user = repo
            .find_user_by_email(&email)
            .await
            .expect("find")
            .expect("user");
        assert_eq!(user.organization_id, legacy_org);
        assert_eq!(user.membership_organization_id, Some(other_org));
        assert_eq!(
            resolve_active_organization_id(user.membership_organization_id, user.organization_id),
            Some(other_org)
        );

        let issued = SessionIssuer.issue_session_token();
        let expires_at = Utc::now() + Duration::days(1);
        repo.create_session(
            user_id,
            &issued.token_prefix,
            &issued.token_hash,
            expires_at,
            Some(other_org),
        )
        .await
        .expect("session");

        let mut session = repo
            .find_session_by_hash(&issued.token_hash)
            .await
            .expect("load")
            .expect("session present");
        assert_eq!(session.active_organization_id, Some(other_org));
        session.is_platform_super_admin =
            effective_platform_super_admin(session.is_platform_super_admin, &session.email, &[]);
        assert!(!session.is_platform_super_admin);

        let _ = sqlx::query("delete from organization where id = $1")
            .bind(other_org)
            .execute(&pool)
            .await;
        cleanup_user(&pool, user_id, legacy_org).await;
    }

    #[tokio::test]
    async fn orphan_principal_fails_require_helpers() {
        // No DB needed: both gates fail closed without super-admin and without home.
        let session = sample_session(None, false);
        assert!(require_active_org(&session).is_err());
        assert!(require_platform_super_admin(&session).is_err());
        let _ = ApiState {
            config: AppConfig::default_local(),
            pool: None,
            retrieval: None,
        };
    }
}
