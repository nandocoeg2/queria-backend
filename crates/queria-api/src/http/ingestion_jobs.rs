use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use queria_core::ids::{IngestionJobId, SourceDocumentId};
use queria_db::ingestion::{IngestionJobRecord, JobMutation, PgIngestionRepository};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_LIMIT: u16 = 50;
const MAX_LIMIT: u16 = 200;

#[derive(Debug, Deserialize)]
struct ListJobsQuery {
    status: Option<String>,
    limit: Option<u16>,
}

#[derive(Debug, Serialize)]
struct IngestionJobResponse {
    id: String,
    project_id: Option<String>,
    source_document_id: Option<String>,
    status: String,
    job_type: String,
    payload: Value,
    locked_by: Option<String>,
    attempts: i32,
    error_message: Option<String>,
    result: Value,
    retry_of_id: Option<String>,
    cancel_requested_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn source_router() -> Router<ApiState> {
    Router::new().route("/{source_document_id}/ingest", post(trigger_ingestion))
}

pub fn job_router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_jobs))
        .route("/{job_id}", get(get_job))
        .route("/{job_id}/retry", post(retry_job))
        .route("/{job_id}/cancel", post(cancel_job))
}

async fn trigger_ingestion(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(source_document_id): Path<SourceDocumentId>,
) -> ApiResult<IngestionJobResponse> {
    let session = require_session(&state, &headers).await?;
    let repository = repository(&state)?;
    let Some(job) = repository
        .trigger(session.user_id, source_document_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "git_source_not_found"));
    };

    Ok(Json(job.into()))
}

async fn list_jobs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListJobsQuery>,
) -> ApiResult<Vec<IngestionJobResponse>> {
    let session = require_session(&state, &headers).await?;
    let status = query.status.as_deref().map(str::trim);
    if status.is_some_and(|status| !valid_status(status)) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_ingestion_status"));
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_limit"));
    }

    let jobs = repository(&state)?
        .list_for_user(session.user_id, status, i64::from(limit))
        .await
        .map_err(map_error)?;
    Ok(Json(jobs.into_iter().map(Into::into).collect()))
}

async fn get_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(job_id): Path<IngestionJobId>,
) -> ApiResult<IngestionJobResponse> {
    let session = require_session(&state, &headers).await?;
    let Some(job) = repository(&state)?
        .get_for_user(session.user_id, job_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "ingestion_job_not_found"));
    };
    Ok(Json(job.into()))
}

async fn retry_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(job_id): Path<IngestionJobId>,
) -> ApiResult<IngestionJobResponse> {
    let session = require_session(&state, &headers).await?;
    mutation_response(
        repository(&state)?
            .retry(session.user_id, job_id)
            .await
            .map_err(map_error)?,
    )
}

async fn cancel_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(job_id): Path<IngestionJobId>,
) -> ApiResult<IngestionJobResponse> {
    let session = require_session(&state, &headers).await?;
    mutation_response(
        repository(&state)?
            .cancel(session.user_id, job_id)
            .await
            .map_err(map_error)?,
    )
}

fn mutation_response(mutation: JobMutation<IngestionJobRecord>) -> ApiResult<IngestionJobResponse> {
    match mutation {
        JobMutation::Updated(job) => Ok(Json(job.into())),
        JobMutation::NotFound => Err(error(StatusCode::NOT_FOUND, "ingestion_job_not_found")),
        JobMutation::InvalidState => Err(error(
            StatusCode::CONFLICT,
            "invalid_ingestion_job_transition",
        )),
    }
}

impl From<IngestionJobRecord> for IngestionJobResponse {
    fn from(value: IngestionJobRecord) -> Self {
        Self {
            id: value.id.to_string(),
            project_id: value.project_id.map(|id| id.to_string()),
            source_document_id: value.source_document_id.map(|id| id.to_string()),
            status: value.status,
            job_type: value.job_type,
            payload: value.payload,
            locked_by: value.locked_by,
            attempts: value.attempts,
            error_message: value.error_message,
            result: value.result,
            retry_of_id: value.retry_of_id.map(|id| id.to_string()),
            cancel_requested_at: value.cancel_requested_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<queria_db::repositories::AuthenticatedSession, (StatusCode, Json<ErrorResponse>)> {
    auth::require_session(state, headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))
}

fn repository(
    state: &ApiState,
) -> Result<PgIngestionRepository, (StatusCode, Json<ErrorResponse>)> {
    state.ingestion_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ingestion_store_not_configured",
        )
    })
}

fn valid_status(status: &str) -> bool {
    matches!(
        status,
        "queued" | "running" | "succeeded" | "failed" | "cancelled"
    )
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "ingestion repository failed");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_limit_is_bounded() {
        assert_eq!(DEFAULT_LIMIT, 50);
        assert_eq!(MAX_LIMIT, 200);
    }

    #[test]
    fn only_known_job_statuses_are_accepted() {
        assert!(valid_status("running"));
        assert!(!valid_status("pending"));
    }
}
