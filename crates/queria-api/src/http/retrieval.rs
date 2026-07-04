use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use queria_core::QueriaError;
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/retrieve-context", post(retrieve_context))
        .route("/retrieval/retrieve-context", post(retrieve_context))
}

async fn retrieve_context(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<RetrieveContextRequest>,
) -> ApiResult<RetrieveContextResponse> {
    let session = auth::require_session(&state, &headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))?;
    request.validate().map_err(map_error)?;

    let Some(pool) = state.pool.clone() else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        ));
    };
    let service = build_pg_retrieval_service(&state.config, pool).map_err(map_error)?;
    service
        .retrieve_context(
            &RetrievalPrincipal::User {
                user_id: session.user_id,
            },
            request,
        )
        .await
        .map(Json)
        .map_err(map_error)
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "retrieval repository failed");
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
