use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use queria_core::QueriaError;
use queria_core::evaluation::EvaluationReport;
use queria_db::evaluation::EvaluationReportRecord;
use queria_search::retrieval::build_pg_retrieval_service;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

const EVALUATION_RETRY_ATTEMPTS: usize = 3;
const EVALUATION_RETRY_DELAY_MS: u64 = 750;

#[derive(Debug, Serialize)]
struct RunEvaluationResponse {
    evaluation: EvaluationReportRecord,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn project_router() -> Router<ApiState> {
    Router::new()
        .route("/{slug}/evaluations/run", post(run_evaluation))
        .route("/{slug}/evaluations", get(list_evaluations))
        .route("/{slug}/evaluations/latest", get(latest_evaluation))
}

async fn run_evaluation(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<RunEvaluationResponse> {
    let session = require_session(&state, &headers).await?;
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
    let report = evaluate_project(&state, session.user_id, &slug, project.id).await?;
    let repository = state.evaluation_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "evaluation_store_not_configured",
        )
    })?;
    let evaluation = repository
        .insert_for_project_slug(session.user_id, &slug, &report)
        .await
        .map_err(map_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "project_not_found"))?;

    Ok(Json(RunEvaluationResponse { evaluation }))
}

async fn list_evaluations(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Vec<EvaluationReportRecord>> {
    let session = require_session(&state, &headers).await?;
    if !valid_slug(&slug) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
    }

    let repository = evaluation_repository(&state)?;
    let evaluations = repository
        .list_for_project_slug(session.user_id, &slug, 25)
        .await
        .map_err(map_error)?;
    Ok(Json(evaluations))
}

async fn latest_evaluation(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<EvaluationReportRecord> {
    let session = require_session(&state, &headers).await?;
    if !valid_slug(&slug) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
    }

    let repository = evaluation_repository(&state)?;
    let Some(evaluation) = repository
        .latest_for_project_slug(session.user_id, &slug)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "evaluation_report_not_found"));
    };
    Ok(Json(evaluation))
}

async fn evaluate_project(
    state: &ApiState,
    user_id: uuid::Uuid,
    project_slug: &str,
    project_id: uuid::Uuid,
) -> Result<EvaluationReport, (StatusCode, Json<ErrorResponse>)> {
    let golden_path = PathBuf::from(format!("tests/golden_questions/{project_slug}.jsonl"));
    if !golden_path.exists() {
        return Err(error(StatusCode::NOT_FOUND, "golden_questions_not_found"));
    }

    let pool = state.pool.clone().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        )
    })?;
    let service = build_pg_retrieval_service(&state.config, pool).map_err(map_error)?;
    let executor = queria_search::evaluation::EvaluationExecutor::new(
        service,
        EVALUATION_RETRY_ATTEMPTS,
        Duration::from_millis(EVALUATION_RETRY_DELAY_MS),
    );

    executor
        .run(
            user_id,
            project_slug,
            queria_core::ids::ProjectId::from_uuid(project_id),
            &golden_path,
        )
        .await
        .map_err(map_error)
}

async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<queria_db::repositories::AuthenticatedSession, (StatusCode, Json<ErrorResponse>)> {
    auth::require_session(state, headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))
}

fn evaluation_repository(
    state: &ApiState,
) -> Result<queria_db::evaluation::PgEvaluationRepository, (StatusCode, Json<ErrorResponse>)> {
    state.evaluation_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "evaluation_store_not_configured",
        )
    })
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

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "evaluation repository failed");
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
    fn test_valid_slug() {
        assert!(valid_slug("fjulian-me"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("a"));
        assert!(!valid_slug("ab"));
        assert!(!valid_slug("-slug"));
        assert!(!valid_slug("slug-"));
    }
}
