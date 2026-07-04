use axum::{Json, Router, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    authenticated: bool,
    error: Option<&'static str>,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/login", post(login))
}

async fn login(Json(payload): Json<LoginRequest>) -> (StatusCode, Json<LoginResponse>) {
    let _redacted_email_present = !payload.email.trim().is_empty();
    let _password_present = !payload.password.is_empty();

    (
        StatusCode::UNAUTHORIZED,
        Json(LoginResponse {
            authenticated: false,
            error: Some("user_store_not_configured"),
        }),
    )
}
