use crate::app::ApiState;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use queria_core::QueriaError;
use queria_core::auth::agent_token::AgentTokenIssuer;
use queria_core::auth::password::PasswordHasher;
use queria_db::repositories::CompleteSetupParams;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct SetupStateResponse {
    setup_required: bool,
    first_admin_email: String,
    organization_slug: String,
}

#[derive(Debug, Deserialize)]
struct CompleteSetupRequest {
    setup_token: String,
    admin_email: Option<String>,
    admin_password: String,
    organization_slug: Option<String>,
    organization_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct CompleteSetupResponse {
    setup_completed: bool,
    organization_slug: Option<String>,
    admin_email: Option<String>,
    user_id: Option<String>,
    error: Option<String>,
}

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/state", get(setup_state))
        .route("/complete", post(complete_setup))
}

async fn setup_state(State(state): State<ApiState>) -> (StatusCode, Json<SetupStateResponse>) {
    let setup_required = match state.auth_repository() {
        Some(repository) => repository.setup_required().await.unwrap_or(true),
        None => true,
    };

    (
        StatusCode::OK,
        Json(SetupStateResponse {
            setup_required,
            first_admin_email: state.config.first_admin_email,
            organization_slug: state.config.first_org_slug,
        }),
    )
}

async fn complete_setup(
    State(state): State<ApiState>,
    Json(payload): Json<CompleteSetupRequest>,
) -> (StatusCode, Json<CompleteSetupResponse>) {
    let Some(repository) = state.auth_repository() else {
        return setup_error(StatusCode::SERVICE_UNAVAILABLE, "database_not_configured");
    };

    if payload.setup_token != state.config.setup_token {
        return setup_error(StatusCode::UNAUTHORIZED, "invalid_setup_token");
    }

    let admin_email = payload
        .admin_email
        .unwrap_or_else(|| state.config.first_admin_email.clone())
        .trim()
        .to_lowercase();
    let organization_slug = payload
        .organization_slug
        .unwrap_or_else(|| state.config.first_org_slug.clone())
        .trim()
        .to_owned();
    let organization_name = payload
        .organization_name
        .unwrap_or_else(|| organization_slug.clone())
        .trim()
        .to_owned();

    if !admin_email.contains('@') {
        return setup_error(StatusCode::BAD_REQUEST, "invalid_admin_email");
    }

    let password_hash = match PasswordHasher.hash_password(&payload.admin_password) {
        Ok(password_hash) => password_hash,
        Err(_) => return setup_error(StatusCode::BAD_REQUEST, "weak_admin_password"),
    };

    let result = repository
        .complete_first_run(CompleteSetupParams {
            organization_slug,
            organization_name,
            admin_email,
            password_hash,
            setup_token_hash: AgentTokenIssuer::hash_token(&payload.setup_token),
        })
        .await;

    match result {
        Ok(created) => (
            StatusCode::CREATED,
            Json(CompleteSetupResponse {
                setup_completed: true,
                organization_slug: Some(created.organization_slug),
                admin_email: Some(created.email),
                user_id: Some(created.user_id.to_string()),
                error: None,
            }),
        ),
        Err(QueriaError::Validation(message)) => setup_error(StatusCode::CONFLICT, &message),
        Err(_) => setup_error(StatusCode::INTERNAL_SERVER_ERROR, "setup_failed"),
    }
}

fn setup_error(status: StatusCode, message: &str) -> (StatusCode, Json<CompleteSetupResponse>) {
    (
        status,
        Json(CompleteSetupResponse {
            setup_completed: false,
            organization_slug: None,
            admin_email: None,
            user_id: None,
            error: Some(message.to_owned()),
        }),
    )
}
