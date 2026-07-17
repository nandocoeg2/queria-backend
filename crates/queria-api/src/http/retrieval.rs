use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use queria_core::QueriaError;
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct RetrievalProbeRequest {
    query: String,
    include_global: Option<bool>,
    /// Operator probe default false (trusted-only); agents default true on MCP.
    include_scratch: Option<bool>,
    limit: Option<u32>,
}

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

pub fn project_router() -> Router<ApiState> {
    Router::new().route("/{slug}/retrieval/probe", post(retrieve_context_by_slug))
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

async fn retrieve_context_by_slug(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<RetrievalProbeRequest>,
) -> ApiResult<RetrieveContextResponse> {
    let session = auth::require_session(&state, &headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))?;
    if !valid_slug(&slug) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
    }

    let project = state
        .project_repository()
        .ok_or_else(|| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "project_store_not_configured",
            )
        })?
        .get_project_by_slug(session.user_id, &slug)
        .await
        .map_err(map_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "project_not_found"))?;
    let request = RetrieveContextRequest {
        project_id: queria_core::ids::ProjectId::from_uuid(project.id),
        query: payload.query,
        include_global: payload
            .include_global
            .unwrap_or(project.include_global_default),
        // Operator slug probe: default trusted-only (VAL-CROSS-007 adjacent).
        include_scratch: payload.include_scratch.unwrap_or(false),
        limit: payload.limit.unwrap_or(5),
        // Surface wiring for request overrides lands with later features.
        rerank: None,
        compress: None,
    };
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

fn valid_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };

    (3..=64).contains(&bytes.len())
        && first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}
