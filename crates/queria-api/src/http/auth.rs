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
use queria_db::repositories::AuthenticatedSession;
use serde::{Deserialize, Serialize};

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
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct MeResponse {
    authenticated: bool,
    user_id: Option<String>,
    email: Option<String>,
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

    let issued = SessionIssuer.issue_session_token();
    let expires_at = Utc::now() + Duration::days(7);
    if repository
        .create_session(
            user.id,
            &issued.token_prefix,
            &issued.token_hash,
            expires_at,
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

    (
        StatusCode::OK,
        headers,
        Json(LoginResponse {
            authenticated: true,
            user_id: Some(user.id.to_string()),
            email: Some(user.email),
            expires_at: Some(expires_at.to_rfc3339()),
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
    let Ok(Some(session)) = repository.find_session_by_hash(&token_hash).await else {
        return Err("invalid_session");
    };

    Ok(session)
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
