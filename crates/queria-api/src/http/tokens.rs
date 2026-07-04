use axum::{Json, Router, http::StatusCode, routing::post};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct CreateAgentTokenResponse {
    token: Option<String>,
    token_prefix: Option<String>,
    error: Option<&'static str>,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/", post(create_agent_token))
}

async fn create_agent_token() -> (StatusCode, Json<CreateAgentTokenResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(CreateAgentTokenResponse {
            token: None,
            token_prefix: None,
            error: Some("admin_session_required"),
        }),
    )
}
