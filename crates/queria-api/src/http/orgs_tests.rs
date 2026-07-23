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
    let stored_hash: String = sqlx::query_scalar("select token_hash from org_invite where id = $1")
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
    let mcount: i64 = sqlx::query_scalar("select count(*) from org_membership where user_id = $1")
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
    let still: Uuid = sqlx::query_scalar("select organization_id from user_account where id = $1")
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
    let (status, _) = json_request(app, "GET", "/api/v1/orgs/current/members", None, None).await;
    assert_eq!(status, HttpStatus::UNAUTHORIZED);
}
