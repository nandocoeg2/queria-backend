use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use queria_core::QueriaError;
use queria_core::ids::{IngestionJobId, ProjectId};
use queria_db::embedding::PgEmbeddingRepository;
use queria_db::ingestion::{IngestionJobRecord, JobMutation};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct JobQuery {
    status: Option<String>,
}

pub fn project_router() -> Router<ApiState> {
    Router::new()
        .route("/{slug}/embedding-jobs/backfill", post(trigger_backfill))
        .route("/{slug}/embedding-jobs", get(list_project_jobs))
        .route("/{slug}/retrieval/status", get(retrieval_status))
}

pub fn job_router() -> Router<ApiState> {
    Router::new()
        .route("/{id}", get(get_job))
        .route("/{id}/retry", post(retry_job))
        .route("/{id}/cancel", post(cancel_job))
}

async fn trigger_backfill(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = require_session(&state, &headers).await?;
    let (project_id, repository) = project_and_repository(&state, session.user_id, &slug).await?;
    let job = repository
        .enqueue_backfill(
            session.user_id,
            project_id,
            &state.config.embedding.profile_version,
        )
        .await
        .map_err(map_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "project_not_found"))?;
    Ok(Json(json!({ "job": job })))
}

async fn list_project_jobs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<JobQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = require_session(&state, &headers).await?;
    let (project_id, _) = project_and_repository(&state, session.user_id, &slug).await?;
    let repository = state.ingestion_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "job_store_not_configured",
        )
    })?;
    let jobs = repository
        .list_for_user(session.user_id, query.status.as_deref(), 100)
        .await
        .map_err(map_error)?
        .into_iter()
        .filter(|job| {
            job.project_id == Some(project_id.as_uuid()) && is_embedding_job(&job.job_type)
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({ "jobs": jobs })))
}

async fn retrieval_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = require_session(&state, &headers).await?;
    let (project_id, repository) = project_and_repository(&state, session.user_id, &slug).await?;
    let counts = repository
        .status_counts(project_id, &state.config.embedding.profile_version)
        .await
        .map_err(map_error)?;
    Ok(Json(json!({
        "project_id": project_id,
        "embedding_profile_version": state.config.embedding.profile_version,
        "counts": counts
    })))
}

async fn get_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = require_session(&state, &headers).await?;
    let repository = state.ingestion_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "job_store_not_configured",
        )
    })?;
    let job = repository
        .get_for_user(session.user_id, IngestionJobId::from_uuid(id))
        .await
        .map_err(map_error)?
        .filter(|job| is_embedding_job(&job.job_type))
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "embedding_job_not_found"))?;
    Ok(Json(json!({ "job": job })))
}

async fn retry_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    mutate_job(&state, &headers, id, true).await
}

async fn cancel_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    mutate_job(&state, &headers, id, false).await
}

async fn mutate_job(
    state: &ApiState,
    headers: &HeaderMap,
    id: Uuid,
    retry: bool,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = require_session(state, headers).await?;
    let repository = state.ingestion_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "job_store_not_configured",
        )
    })?;
    let typed_id = IngestionJobId::from_uuid(id);
    let Some(existing) = repository
        .get_for_user(session.user_id, typed_id)
        .await
        .map_err(map_error)?
        .filter(|job| is_embedding_job(&job.job_type))
    else {
        return Err(error(StatusCode::NOT_FOUND, "embedding_job_not_found"));
    };
    let mutation = if retry {
        repository.retry(session.user_id, typed_id).await
    } else {
        repository.cancel(session.user_id, typed_id).await
    }
    .map_err(map_error)?;
    mutation_response(mutation, existing)
}

fn mutation_response(
    mutation: JobMutation<IngestionJobRecord>,
    _existing: IngestionJobRecord,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match mutation {
        JobMutation::Updated(job) => Ok(Json(json!({ "job": job }))),
        JobMutation::NotFound => Err(error(StatusCode::NOT_FOUND, "embedding_job_not_found")),
        JobMutation::InvalidState => Err(error(
            StatusCode::CONFLICT,
            "invalid_embedding_job_transition",
        )),
    }
}

async fn project_and_repository(
    state: &ApiState,
    user_id: Uuid,
    slug: &str,
) -> Result<(ProjectId, PgEmbeddingRepository), (StatusCode, Json<Value>)> {
    let project = state
        .project_repository()
        .ok_or_else(|| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "project_store_not_configured",
            )
        })?
        .get_project_by_slug(user_id, slug)
        .await
        .map_err(map_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "project_not_found"))?;
    let pool = state.pool.clone().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "job_store_not_configured",
        )
    })?;
    Ok((
        ProjectId::from_uuid(project.id),
        PgEmbeddingRepository::new(pool),
    ))
}

async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<queria_db::repositories::AuthenticatedSession, (StatusCode, Json<Value>)> {
    auth::require_session(state, headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))
}

fn is_embedding_job(job_type: &str) -> bool {
    matches!(job_type, "embedding_backfill" | "qdrant_delete")
}

fn map_error(error_value: QueriaError) -> (StatusCode, Json<Value>) {
    match error_value {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "permission_denied")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "embedding job repository failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        }
    }
}

fn error(status: StatusCode, message: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message })))
}
