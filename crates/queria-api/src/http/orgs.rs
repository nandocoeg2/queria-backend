//! Organization, invite, and membership HTTP surfaces (multi-org isolation MVP).
//!
//! Routes:
//! - POST/GET  /api/v1/orgs                      — platform super-admin
//! - POST      /api/v1/orgs/{slug}/invites        — org_admin of slug or super-admin
//! - GET       /api/v1/orgs/current/members      — active org
//! - POST      /api/v1/invites/accept            — public

use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use queria_core::QueriaError;
use queria_core::auth::org_invite::OrgInviteTokenIssuer;
use queria_core::auth::password::PasswordHasher;
use queria_db::repositories::{
    AcceptOrgInviteParams, AuthenticatedSession, CreateOrgInviteParams, CreateOrganizationParams,
    OrgInviteRecord, OrgMemberRecord, OrganizationRecord, PgOrgRepository,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const INVITE_TTL_DAYS: i64 = 7;

#[derive(Debug, Deserialize)]
struct CreateOrgRequest {
    slug: String,
    name: String,
    first_admin_email: String,
}

#[derive(Debug, Deserialize)]
struct CreateInviteRequest {
    email: String,
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AcceptInviteRequest {
    token: String,
    password: String,
    /// Optional display name for new users (ignored by v1 storage; accepted for forward compat).
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct OrganizationResponse {
    id: String,
    slug: String,
    name: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct CreateOrgResponse {
    organization: OrganizationResponse,
    /// One-time raw invite token; never stored, never re-fetched.
    invite_token: String,
    invite: InviteMetaResponse,
}

#[derive(Debug, Serialize)]
struct CreateInviteResponse {
    invite_token: String,
    invite: InviteMetaResponse,
}

#[derive(Debug, Serialize)]
struct InviteMetaResponse {
    id: String,
    email: String,
    role: String,
    token_prefix: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
struct MemberResponse {
    user_id: String,
    email: String,
    role: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct AcceptInviteResponse {
    accepted: bool,
    user_id: Option<String>,
    email: Option<String>,
    organization_id: Option<String>,
    organization_slug: Option<String>,
    role: Option<String>,
    created_user: Option<bool>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/orgs", get(list_orgs).post(create_org))
        .route("/api/v1/orgs/{slug}/invites", post(create_invite))
        .route("/api/v1/orgs/current/members", get(list_current_members))
        .route("/api/v1/invites/accept", post(accept_invite))
}

async fn list_orgs(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<OrganizationResponse>> {
    let session = require_session(&state, &headers).await?;
    auth::require_platform_super_admin(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;

    let repository = org_repository(&state)?;
    let orgs = repository.list_organizations().await.map_err(map_error)?;
    Ok(Json(
        orgs.into_iter().map(OrganizationResponse::from).collect(),
    ))
}

async fn create_org(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<CreateOrgRequest>,
) -> ApiResult<CreateOrgResponse> {
    let session = require_session(&state, &headers).await?;
    auth::require_platform_super_admin(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;

    let slug = payload.slug.trim().to_owned();
    let name = payload.name.trim().to_owned();
    let first_admin_email = normalize_email(&payload.first_admin_email);

    if !valid_org_slug(&slug) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_organization_slug"));
    }
    if name.is_empty() || name.len() > 128 {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_organization_name"));
    }
    if !valid_email(&first_admin_email) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_first_admin_email"));
    }

    let issued = OrgInviteTokenIssuer.issue().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invite_token_issue_failed",
        )
    })?;
    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);

    let repository = org_repository(&state)?;
    let (organization, invite) = repository
        .create_organization_with_invite(CreateOrganizationParams {
            slug,
            name,
            first_admin_email,
            token_prefix: issued.token_prefix,
            token_hash: issued.token_hash,
            expires_at,
            invited_by_user_id: session.user_id,
        })
        .await
        .map_err(map_create_org_error)?;

    // Never log raw token at info — optional accept path only.
    tracing::info!(
        organization_slug = %organization.slug,
        email = %invite.email,
        invite_id = %invite.id,
        accept_path = "/admin/invites/accept",
        "org invite created"
    );

    Ok(Json(CreateOrgResponse {
        organization: OrganizationResponse::from(organization),
        invite_token: issued.raw_token,
        invite: InviteMetaResponse::from(invite),
    }))
}

async fn create_invite(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<CreateInviteRequest>,
) -> ApiResult<CreateInviteResponse> {
    let session = require_session(&state, &headers).await?;
    let repository = org_repository(&state)?;

    let org = repository
        .get_organization_by_slug(slug.trim())
        .await
        .map_err(map_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "organization_not_found"))?;

    authorize_invite_create(&session, &repository, org.id).await?;

    let email = normalize_email(&payload.email);
    if !valid_email(&email) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_invite_email"));
    }

    let role = payload
        .role
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("org_member")
        .to_owned();
    if role != "org_admin" && role != "org_member" {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_invite_role"));
    }

    let issued = OrgInviteTokenIssuer.issue().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invite_token_issue_failed",
        )
    })?;
    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);

    let invite = repository
        .create_invite(CreateOrgInviteParams {
            organization_id: org.id,
            email,
            role,
            token_prefix: issued.token_prefix,
            token_hash: issued.token_hash,
            expires_at,
            invited_by_user_id: session.user_id,
        })
        .await
        .map_err(map_error)?;

    tracing::info!(
        organization_slug = %org.slug,
        email = %invite.email,
        invite_id = %invite.id,
        accept_path = "/admin/invites/accept",
        "org invite created"
    );

    Ok(Json(CreateInviteResponse {
        invite_token: issued.raw_token,
        invite: InviteMetaResponse::from(invite),
    }))
}

async fn list_current_members(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<MemberResponse>> {
    let session = require_session(&state, &headers).await?;
    let home = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;

    let repository = org_repository(&state)?;
    let members = repository.list_members(home).await.map_err(map_error)?;
    Ok(Json(
        members.into_iter().map(MemberResponse::from).collect(),
    ))
}

async fn accept_invite(
    State(state): State<ApiState>,
    Json(payload): Json<AcceptInviteRequest>,
) -> (StatusCode, Json<AcceptInviteResponse>) {
    let Some(repository) = state.org_repository() else {
        return accept_error(StatusCode::SERVICE_UNAVAILABLE, "database_not_configured");
    };

    let token = payload.token.trim();
    if token.is_empty() {
        return accept_error(StatusCode::BAD_REQUEST, "invalid_invite_token");
    }

    // Product password rule matches setup: non-empty and >= 12 (PasswordHasher).
    if payload.password.is_empty() {
        return accept_error(StatusCode::BAD_REQUEST, "empty_password");
    }

    let password_hash = match PasswordHasher.hash_password(&payload.password) {
        Ok(hash) => hash,
        Err(QueriaError::Validation(_)) => {
            return accept_error(StatusCode::BAD_REQUEST, "weak_password");
        }
        Err(_) => return accept_error(StatusCode::INTERNAL_SERVER_ERROR, "password_hash_failed"),
    };

    let token_hash = OrgInviteTokenIssuer::hash_token(token);
    // Keep optional name from body used only so empty struct fields do not warn.
    let _name = payload.name;

    match repository
        .accept_invite(AcceptOrgInviteParams {
            token_hash,
            password_hash,
        })
        .await
    {
        Ok(accepted) => (
            StatusCode::OK,
            Json(AcceptInviteResponse {
                accepted: true,
                user_id: Some(accepted.user_id.to_string()),
                email: Some(accepted.email),
                organization_id: Some(accepted.organization_id.to_string()),
                organization_slug: Some(accepted.organization_slug),
                role: Some(accepted.role),
                created_user: Some(accepted.created_user),
                error: None,
            }),
        ),
        Err(QueriaError::Validation(message)) => {
            let status = match message.as_str() {
                "invite_invalid" | "invite_revoked" => StatusCode::BAD_REQUEST,
                "invite_already_used" => StatusCode::CONFLICT,
                "invite_expired" => StatusCode::GONE,
                "already_member_of_other_org" => StatusCode::CONFLICT,
                _ => StatusCode::BAD_REQUEST,
            };
            accept_error(status, &message)
        }
        Err(_) => accept_error(StatusCode::INTERNAL_SERVER_ERROR, "accept_failed"),
    }
}

async fn authorize_invite_create(
    session: &AuthenticatedSession,
    repository: &PgOrgRepository,
    organization_id: Uuid,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if session.is_platform_super_admin {
        return Ok(());
    }

    let Some(role) = repository
        .membership_role(session.user_id, organization_id)
        .await
        .map_err(map_error)?
    else {
        // Hide existence for foreign orgs when caller is not super-admin.
        return Err(error(StatusCode::FORBIDDEN, "invite_create_forbidden"));
    };

    // v1: humans with membership are org_admin-powered; still require org_admin role string
    // for invite create (org_member may exist later with different powers).
    if role != "org_admin" {
        return Err(error(StatusCode::FORBIDDEN, "invite_create_forbidden"));
    }

    Ok(())
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn valid_email(email: &str) -> bool {
    let at = match email.find('@') {
        Some(idx) if idx > 0 && idx < email.len() - 1 => idx,
        _ => return false,
    };
    email[at + 1..].contains('.')
}

fn valid_org_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };

    // Matches organization.slug check: ^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$
    (3..=64).contains(&bytes.len())
        && first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, (StatusCode, Json<ErrorResponse>)> {
    auth::require_session(state, headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))
}

fn org_repository(state: &ApiState) -> Result<PgOrgRepository, (StatusCode, Json<ErrorResponse>)> {
    state.org_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "org_store_not_configured",
        )
    })
}

fn map_create_org_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) if message == "organization_slug_exists" => {
            error(StatusCode::CONFLICT, "organization_slug_exists")
        }
        other => map_error(other),
    }
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::FORBIDDEN, "permission_denied")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "org repository failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        }
    }
}

fn error(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: message.to_owned(),
        }),
    )
}

fn accept_error(status: StatusCode, message: &str) -> (StatusCode, Json<AcceptInviteResponse>) {
    (
        status,
        Json(AcceptInviteResponse {
            accepted: false,
            user_id: None,
            email: None,
            organization_id: None,
            organization_slug: None,
            role: None,
            created_user: None,
            error: Some(message.to_owned()),
        }),
    )
}

impl From<OrganizationRecord> for OrganizationResponse {
    fn from(value: OrganizationRecord) -> Self {
        Self {
            id: value.id.to_string(),
            slug: value.slug,
            name: value.name,
            created_at: value.created_at.to_rfc3339(),
        }
    }
}

impl From<OrgInviteRecord> for InviteMetaResponse {
    fn from(value: OrgInviteRecord) -> Self {
        Self {
            id: value.id.to_string(),
            email: value.email,
            role: value.role,
            token_prefix: value.token_prefix,
            expires_at: value.expires_at.to_rfc3339(),
        }
    }
}

impl From<OrgMemberRecord> for MemberResponse {
    fn from(value: OrgMemberRecord) -> Self {
        Self {
            user_id: value.user_id.to_string(),
            email: value.email,
            role: value.role,
            created_at: value.created_at.to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::build_app_with_pool;
    use ::http::{Request, StatusCode as HttpStatus, header};
    use axum::body::{Body, to_bytes};
    use queria_core::AppConfig;
    use queria_core::auth::org_invite::OrgInviteTokenIssuer;
    use queria_core::auth::password::PasswordHasher;
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;

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
    ) -> (Uuid, Uuid, String) {
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

        (user_id, org_id, slug)
    }

    async fn cleanup_user(pool: &PgPool, user_id: Uuid, org_id: Uuid) {
        let _ = sqlx::query("delete from user_session where user_id = $1")
            .bind(user_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from org_invite where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from org_membership where user_id = $1")
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

    async fn cleanup_org_by_slug(pool: &PgPool, slug: &str) {
        let Ok(Some(org_id)): Result<Option<Uuid>, _> =
            sqlx::query_scalar("select id from organization where slug = $1")
                .bind(slug)
                .fetch_optional(pool)
                .await
        else {
            return;
        };
        let _ = sqlx::query(
            "delete from user_session where user_id in (
               select id from user_account where organization_id = $1
             )",
        )
        .bind(org_id)
        .execute(pool)
        .await;
        let _ = sqlx::query("delete from org_invite where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from org_membership where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from user_account where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from organization where id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
    }

    async fn login_cookie(app: axum::Router, email: &str, password: &str) -> (String, Value) {
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
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("set-cookie")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_owned();
        let body = to_bytes(response.into_body(), 1024 * 64)
            .await
            .expect("body");
        let json: Value = serde_json::from_slice(&body).expect("json");
        (cookie, json)
    }

    async fn json_request(
        app: axum::Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<&str>,
    ) -> (HttpStatus, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let response = app
            .oneshot(
                builder
                    .body(Body::from(body.unwrap_or("").to_owned()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 256)
            .await
            .expect("body");
        let json: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    #[test]
    fn valid_org_slug_matches_schema_regex() {
        assert!(valid_org_slug("team-b"));
        assert!(valid_org_slug("ab1"));
        assert!(!valid_org_slug("ab"));
        assert!(!valid_org_slug("-ab"));
        assert!(!valid_org_slug("AB"));
        assert!(!valid_org_slug(""));
    }

    #[test]
    fn normalize_email_lowercases() {
        assert_eq!(normalize_email(" Admin@X.COM "), "admin@x.com");
    }

    /// VAL-ORGS-005 / VAL-ORGS-029: unauthenticated create/list denied.
    #[tokio::test]
    async fn unauthenticated_orgs_routes_denied() {
        let app = crate::app::build_app(AppConfig::default_local());
        let (status, _) = json_request(app.clone(), "GET", "/api/v1/orgs", None, None).await;
        assert_eq!(status, HttpStatus::UNAUTHORIZED);

        let (status, _) = json_request(
            app,
            "POST",
            "/api/v1/orgs",
            None,
            Some(r#"{"slug":"x","name":"X","first_admin_email":"a@b.co"}"#),
        )
        .await;
        assert_eq!(status, HttpStatus::UNAUTHORIZED);
    }

    /// VAL-ORGS-001,002,003,006,012,031,032
    #[tokio::test]
    async fn super_admin_create_and_list_orgs_token_once_hashed() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let password = "correct horse battery staple";
        let email = format!("super-{}@orgs.test", Uuid::now_v7().simple());
        let (user_id, home_org, _) = seed_user(&pool, &email, password, false, true).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let (cookie, _) = login_cookie(app.clone(), &email, password).await;

        let slug = format!("team-{}", Uuid::now_v7().simple());
        let admin_email = format!("Admin+{}@TeamB.Example", Uuid::now_v7().simple());
        let (status, create_body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&cookie),
            Some(&format!(
                r#"{{"slug":"{slug}","name":"Team B","first_admin_email":"{admin_email}"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "create: {create_body}");
        let token = create_body["invite_token"].as_str().expect("token");
        assert!(token.starts_with("qinv_"));
        let invite_id = create_body["invite"]["id"].as_str().expect("invite id");
        // Email normalized on create.
        assert_eq!(
            create_body["invite"]["email"].as_str(),
            Some(admin_email.to_ascii_lowercase().as_str())
        );

        // Stored hash only — raw token not at rest.
        let stored_hash: String =
            sqlx::query_scalar("select token_hash from org_invite where id = $1")
                .bind(Uuid::parse_str(invite_id).unwrap())
                .fetch_one(&pool)
                .await
                .expect("hash");
        assert_ne!(stored_hash, token);
        assert_eq!(stored_hash, OrgInviteTokenIssuer::hash_token(token));
        let raw_in_row: bool = sqlx::query_scalar(
            "select exists(
               select 1 from org_invite
               where id = $1 and (token_hash = $2 or token_prefix = $2)
             )",
        )
        .bind(Uuid::parse_str(invite_id).unwrap())
        .bind(token)
        .fetch_one(&pool)
        .await
        .expect("raw check");
        assert!(!raw_in_row);

        // List orgs includes new slug; no invite token fields.
        let (status, list_body) =
            json_request(app.clone(), "GET", "/api/v1/orgs", Some(&cookie), None).await;
        assert_eq!(status, HttpStatus::OK);
        let list = list_body.as_array().expect("array");
        assert!(
            list.iter().any(|o| o["slug"] == slug),
            "list missing {slug}: {list_body}"
        );
        let list_str = list_body.to_string();
        assert!(
            !list_str.contains(token),
            "list must not re-expose raw token"
        );

        // Duplicate slug rejected without orphan second invite.
        let (status, dup) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&cookie),
            Some(&format!(
                r#"{{"slug":"{slug}","name":"Again","first_admin_email":"other@example.com"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::CONFLICT);
        assert_eq!(dup["error"], "organization_slug_exists");

        // Invalid bodies 4xx without orphans.
        let bad_slug = format!("BAD_SLUG_{}", Uuid::now_v7().simple());
        let (status, _) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&cookie),
            Some(&format!(
                r#"{{"slug":"{bad_slug}","name":"X","first_admin_email":"ok@example.com"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::BAD_REQUEST);
        let exists: bool =
            sqlx::query_scalar("select exists(select 1 from organization where slug = $1)")
                .bind(&bad_slug)
                .fetch_one(&pool)
                .await
                .expect("exists");
        assert!(!exists);

        let (status, _) = json_request(
            app,
            "POST",
            "/api/v1/orgs",
            Some(&cookie),
            Some(r#"{"slug":"ok-slug-abc","name":"","first_admin_email":"not-an-email"}"#),
        )
        .await;
        assert_eq!(status, HttpStatus::BAD_REQUEST);

        cleanup_org_by_slug(&pool, &slug).await;
        cleanup_user(&pool, user_id, home_org).await;
    }

    /// VAL-ORGS-004,007
    #[tokio::test]
    async fn non_super_admin_cannot_create_or_list_orgs() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let password = "correct horse battery staple";
        let email = format!("member-{}@orgs.test", Uuid::now_v7().simple());
        let (user_id, org_id, _) = seed_user(&pool, &email, password, true, false).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let (cookie, _) = login_cookie(app.clone(), &email, password).await;

        let slug = format!("denied-{}", Uuid::now_v7().simple());
        let (status, _) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&cookie),
            Some(&format!(
                r#"{{"slug":"{slug}","name":"Nope","first_admin_email":"x@y.com"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN);

        let exists: bool =
            sqlx::query_scalar("select exists(select 1 from organization where slug = $1)")
                .bind(&slug)
                .fetch_one(&pool)
                .await
                .expect("exists");
        assert!(!exists);

        let (status, body) = json_request(app, "GET", "/api/v1/orgs", Some(&cookie), None).await;
        assert_eq!(status, HttpStatus::FORBIDDEN);
        assert!(!body.is_array());

        cleanup_user(&pool, user_id, org_id).await;
    }

    /// VAL-ORGS-008,009,010,011
    #[tokio::test]
    async fn invite_create_authorization_and_token_once() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let password = "correct horse battery staple";

        let super_email = format!("super-{}@orgs.test", Uuid::now_v7().simple());
        let (super_id, super_org, _) = seed_user(&pool, &super_email, password, false, true).await;

        let admin_email = format!("admin-{}@orgs.test", Uuid::now_v7().simple());
        let (admin_id, admin_org, admin_slug) =
            seed_user(&pool, &admin_email, password, true, false).await;

        let foreign_email = format!("foreign-{}@orgs.test", Uuid::now_v7().simple());
        let (foreign_id, foreign_org, _) =
            seed_user(&pool, &foreign_email, password, true, false).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let (super_cookie, _) = login_cookie(app.clone(), &super_email, password).await;
        let (admin_cookie, _) = login_cookie(app.clone(), &admin_email, password).await;
        let (foreign_cookie, _) = login_cookie(app.clone(), &foreign_email, password).await;

        // Org admin can invite into own org.
        let invitee = format!("invitee-{}@example.com", Uuid::now_v7().simple());
        let (status, first) = json_request(
            app.clone(),
            "POST",
            &format!("/api/v1/orgs/{admin_slug}/invites"),
            Some(&admin_cookie),
            Some(&format!(r#"{{"email":"{invitee}","role":"org_member"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "{first}");
        let token1 = first["invite_token"].as_str().unwrap().to_owned();

        // Second invite returns new token only (not prior plaintext).
        let invitee2 = format!("invitee2-{}@example.com", Uuid::now_v7().simple());
        let (status, second) = json_request(
            app.clone(),
            "POST",
            &format!("/api/v1/orgs/{admin_slug}/invites"),
            Some(&admin_cookie),
            Some(&format!(r#"{{"email":"{invitee2}","role":"org_admin"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let token2 = second["invite_token"].as_str().unwrap();
        assert_ne!(token1, token2);
        assert!(!second.to_string().contains(&token1));

        // Super-admin can invite into any org.
        let invitee3 = format!("invitee3-{}@example.com", Uuid::now_v7().simple());
        let (status, _) = json_request(
            app.clone(),
            "POST",
            &format!("/api/v1/orgs/{admin_slug}/invites"),
            Some(&super_cookie),
            Some(&format!(r#"{{"email":"{invitee3}"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::OK);

        // Foreign org admin cannot invite into admin's org.
        let (status, _) = json_request(
            app,
            "POST",
            &format!("/api/v1/orgs/{admin_slug}/invites"),
            Some(&foreign_cookie),
            Some(r#"{"email":"x@y.com"}"#),
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN);

        cleanup_user(&pool, foreign_id, foreign_org).await;
        cleanup_user(&pool, admin_id, admin_org).await;
        cleanup_user(&pool, super_id, super_org).await;
    }

    /// VAL-ORGS-013,014,015,016,017,018,033,034,037
    #[tokio::test]
    async fn accept_invite_flow_and_rejections() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let password = "correct horse battery staple";
        let super_email = format!("super-{}@orgs.test", Uuid::now_v7().simple());
        let (super_id, super_org, _) = seed_user(&pool, &super_email, password, false, true).await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let (super_cookie, _) = login_cookie(app.clone(), &super_email, password).await;

        // Team B create + accept new user.
        let team_b = format!("team-b-{}", Uuid::now_v7().simple());
        let admin_b = format!("AdminB+{}@Example.COM", Uuid::now_v7().simple());
        let (status, created) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&super_cookie),
            Some(&format!(
                r#"{{"slug":"{team_b}","name":"Team B","first_admin_email":"{admin_b}"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let token_b = created["invite_token"].as_str().unwrap().to_owned();
        let org_b_id = created["organization"]["id"].as_str().unwrap().to_owned();

        // Empty password rejected.
        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(r#"{{"token":"{token_b}","password":""}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::BAD_REQUEST);
        assert_eq!(body["error"], "empty_password");

        // Weak password rejected.
        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(r#"{{"token":"{token_b}","password":"short"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::BAD_REQUEST);
        assert_eq!(body["error"], "weak_password");

        // Invalid token rejected.
        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(r#"{"token":"qinv_notreal","password":"correct horse battery staple"}"#),
        )
        .await;
        assert_eq!(status, HttpStatus::BAD_REQUEST);
        assert_eq!(body["error"], "invite_invalid");

        // Happy path accept (case-insensitive email normalize on login later).
        let (status, accepted) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{token_b}","password":"correct horse battery staple"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "{accepted}");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["created_user"], true);
        assert_eq!(
            accepted["organization_id"].as_str(),
            Some(org_b_id.as_str())
        );

        // Membership + organization_id synced.
        let email_norm = admin_b.to_ascii_lowercase();
        let (user_id, org_id, has_membership): (Uuid, Uuid, bool) = {
            let row = sqlx::query(
                "select u.id, u.organization_id,
                        exists(select 1 from org_membership m where m.user_id = u.id) as has_m
                 from user_account u
                 where lower(u.email) = lower($1)",
            )
            .bind(&email_norm)
            .fetch_one(&pool)
            .await
            .expect("user");
            use sqlx::Row;
            (row.get("id"), row.get("organization_id"), row.get("has_m"))
        };
        assert!(has_membership);
        assert_eq!(org_id.to_string(), org_b_id);

        // Login binds active org.
        let (cookie_b, login_json) = login_cookie(app.clone(), &email_norm, password).await;
        assert_eq!(
            login_json["active_organization_id"].as_str(),
            Some(org_b_id.as_str())
        );

        // Used token rejected (second accept).
        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{token_b}","password":"correct horse battery staple"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::CONFLICT);
        assert_eq!(body["error"], "invite_already_used");

        // Same-org re-accept via a fresh invite: create invite for same email, accept.
        let (status, reinvite) = json_request(
            app.clone(),
            "POST",
            &format!("/api/v1/orgs/{team_b}/invites"),
            Some(&super_cookie),
            Some(&format!(r#"{{"email":"{email_norm}","role":"org_admin"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let reinvite_token = reinvite["invite_token"].as_str().unwrap();
        let (status, reaccepted) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{reinvite_token}","password":"correct horse battery staple"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "{reaccepted}");
        // Still single membership.
        let mcount: i64 =
            sqlx::query_scalar("select count(*) from org_membership where user_id = $1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(mcount, 1);

        // Second-org invite rejected for existing Team B user.
        let team_c = format!("team-c-{}", Uuid::now_v7().simple());
        let (status, created_c) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&super_cookie),
            Some(&format!(
                r#"{{"slug":"{team_c}","name":"Team C","first_admin_email":"{email_norm}"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let token_c = created_c["invite_token"].as_str().unwrap();
        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{token_c}","password":"correct horse battery staple"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::CONFLICT);
        assert_eq!(body["error"], "already_member_of_other_org");
        // org_id unchanged
        let still: Uuid =
            sqlx::query_scalar("select organization_id from user_account where id = $1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .expect("org");
        assert_eq!(still.to_string(), org_b_id);

        // Expired invite rejected.
        let issued = OrgInviteTokenIssuer.issue().expect("issue");
        let expired_at = Utc::now() - Duration::hours(1);
        // Need expires_at > created_at at insert; then move expires_at into the past.
        let invite_id: Uuid = sqlx::query_scalar(
            "insert into org_invite(
               organization_id, email, role, token_hash, token_prefix,
               invited_by_user_id, expires_at, created_at
             )
             values ($1, $2, 'org_member', $3, $4, $5, now() + interval '1 day', now())
             returning id",
        )
        .bind(Uuid::parse_str(&org_b_id).unwrap())
        .bind(format!("expired-{}@example.com", Uuid::now_v7().simple()))
        .bind(&issued.token_hash)
        .bind(&issued.token_prefix)
        .bind(super_id)
        .fetch_one(&pool)
        .await
        .expect("expired invite");
        sqlx::query(
            "update org_invite set expires_at = $1, created_at = $1 - interval '1 hour'
             where id = $2",
        )
        .bind(expired_at)
        .bind(invite_id)
        .execute(&pool)
        .await
        .expect("force expire");

        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{}","password":"correct horse battery staple"}}"#,
                issued.raw_token
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::GONE);
        assert_eq!(body["error"], "invite_expired");

        // Revoked invite rejected.
        let issued2 = OrgInviteTokenIssuer.issue().expect("issue");
        let revoke_id: Uuid = sqlx::query_scalar(
            "insert into org_invite(
               organization_id, email, role, token_hash, token_prefix,
               invited_by_user_id, expires_at, revoked_at
             )
             values ($1, $2, 'org_member', $3, $4, $5, now() + interval '1 day', now())
             returning id",
        )
        .bind(Uuid::parse_str(&org_b_id).unwrap())
        .bind(format!("revoked-{}@example.com", Uuid::now_v7().simple()))
        .bind(&issued2.token_hash)
        .bind(&issued2.token_prefix)
        .bind(super_id)
        .fetch_one(&pool)
        .await
        .expect("revoked invite");
        let _ = revoke_id;
        let (status, body) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{}","password":"correct horse battery staple"}}"#,
                issued2.raw_token
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::BAD_REQUEST);
        assert_eq!(body["error"], "invite_revoked");

        // Members list home-only.
        let (status, members) = json_request(
            app.clone(),
            "GET",
            "/api/v1/orgs/current/members",
            Some(&cookie_b),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let arr = members.as_array().expect("members");
        assert!(
            arr.iter().any(|m| m["email"] == email_norm),
            "members must include accepting admin: {members}"
        );
        // Home-org only: every listed email belongs to user_account rows with this org.
        for member in arr {
            let mid = member["user_id"].as_str().expect("user_id");
            let member_org: Uuid =
                sqlx::query_scalar("select organization_id from user_account where id = $1")
                    .bind(Uuid::parse_str(mid).unwrap())
                    .fetch_one(&pool)
                    .await
                    .expect("member org");
            assert_eq!(member_org.to_string(), org_b_id);
        }

        // Super-admin without active org cannot list members.
        let (status, _) = json_request(
            app,
            "GET",
            "/api/v1/orgs/current/members",
            Some(&super_cookie),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN);

        cleanup_org_by_slug(&pool, &team_c).await;
        cleanup_org_by_slug(&pool, &team_b).await;
        cleanup_user(&pool, super_id, super_org).await;
    }

    /// VAL-ORGS-019,020,029 members require active org / session.
    #[tokio::test]
    async fn members_list_requires_session_and_active_org() {
        let app = crate::app::build_app(AppConfig::default_local());
        let (status, _) =
            json_request(app, "GET", "/api/v1/orgs/current/members", None, None).await;
        assert_eq!(status, HttpStatus::UNAUTHORIZED);
    }
}
