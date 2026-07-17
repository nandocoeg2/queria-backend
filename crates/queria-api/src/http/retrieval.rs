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
use queria_search::retrieval::{PgRetrievalService, RetrievalPrincipal};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct RetrievalProbeRequest {
    query: String,
    include_global: Option<bool>,
    /// Operator probe default false (trusted-only); agents default true on MCP.
    include_scratch: Option<bool>,
    limit: Option<u32>,
    /// `None` uses server `QUERIA_RERANK_ENABLED` default.
    rerank: Option<bool>,
    /// `None` uses server `QUERIA_COMPRESS_ENABLED` default.
    compress: Option<bool>,
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

fn retrieval_service(
    state: &ApiState,
) -> Result<&Arc<PgRetrievalService>, (StatusCode, Json<ErrorResponse>)> {
    state.retrieval.as_ref().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        )
    })
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

    let service = retrieval_service(&state)?;
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
    // Operator slug probe: default trusted-only (VAL-CROSS-005). Agent MCP default remains true.
    let request = RetrieveContextRequest {
        project_id: queria_core::ids::ProjectId::from_uuid(project.id),
        query: payload.query,
        include_global: payload
            .include_global
            .unwrap_or(project.include_global_default),
        include_scratch: payload.include_scratch.unwrap_or(false),
        limit: payload.limit.unwrap_or(5),
        rerank: payload.rerank,
        compress: payload.compress,
    };
    request.validate().map_err(map_error)?;

    let service = retrieval_service(&state)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::ids::ProjectId;
    use uuid::Uuid;

    /// VAL-CROSS-005: operator probe body without include_scratch → false.
    #[test]
    fn probe_defaults_include_scratch_false() {
        let payload: RetrievalProbeRequest =
            serde_json::from_str(r#"{"query":"deployment notes","include_global":true,"limit":5}"#)
                .expect("probe body deserializes");
        assert!(!payload.include_scratch.unwrap_or(false));
        assert!(payload.rerank.is_none());
        assert!(payload.compress.is_none());
    }

    /// VAL-CROSS-001 / VAL-CROSS-002: probe accepts explicit rerank/compress overrides.
    #[test]
    fn probe_accepts_rerank_compress_flags() {
        let payload: RetrievalProbeRequest = serde_json::from_str(
            r#"{
                "query": "q",
                "include_scratch": true,
                "rerank": false,
                "compress": false
            }"#,
        )
        .expect("probe body with flags");
        assert_eq!(payload.include_scratch, Some(true));
        assert_eq!(payload.rerank, Some(false));
        assert_eq!(payload.compress, Some(false));
    }

    /// Flags on RetrieveContextRequest pass through (session dual routes share this type).
    #[test]
    fn retrieve_context_request_flags_passthrough() {
        let req: RetrieveContextRequest = serde_json::from_str(
            r#"{
                "project_id": "019083a0-0000-7000-8000-000000000001",
                "query": "notes",
                "include_global": true,
                "limit": 5,
                "rerank": true,
                "compress": false
            }"#,
        )
        .expect("session retrieve body");
        // Omitted include_scratch still agent/serde default true on this contract type.
        assert!(req.include_scratch);
        assert_eq!(req.rerank, Some(true));
        assert_eq!(req.compress, Some(false));
        assert_eq!(
            req.project_id,
            ProjectId::from_uuid(
                Uuid::parse_str("019083a0-0000-7000-8000-000000000001").expect("uuid")
            )
        );
    }
}
