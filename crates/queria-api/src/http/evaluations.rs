use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use queria_core::QueriaError;
use queria_core::contracts::{RetrievalMode, RetrieveContextRequest, RetrieveContextResponse};
use queria_core::evaluation::{
    EvaluationReport, evaluation_limit, parse_golden_questions_jsonl, score_evaluation_report,
};
use queria_db::evaluation::EvaluationReportRecord;
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};
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
    let content = std::fs::read_to_string(&golden_path).map_err(|read_error| {
        tracing::warn!(
            error = %read_error,
            project_slug = project_slug,
            "golden question file unavailable"
        );
        error(StatusCode::NOT_FOUND, "golden_questions_not_found")
    })?;
    let questions = parse_golden_questions_jsonl(&content)
        .map_err(map_error)?
        .into_iter()
        .filter(|question| question.project_slug == project_slug)
        .collect::<Vec<_>>();
    if questions.is_empty() {
        return Err(error(StatusCode::NOT_FOUND, "golden_questions_not_found"));
    }

    let pool = state.pool.clone().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        )
    })?;
    let service = build_pg_retrieval_service(&state.config, pool).map_err(map_error)?;
    let mut responses = Vec::with_capacity(questions.len());
    for question in &questions {
        let request = RetrieveContextRequest {
            project_id: queria_core::ids::ProjectId::from_uuid(project_id),
            query: question.query.clone(),
            include_global: question.include_global,
            limit: evaluation_limit(question.minimum_items),
        };
        let response = retrieve_for_evaluation(&service, user_id, request).await;
        responses.push(response);
    }

    Ok(score_evaluation_report(
        project_slug,
        &golden_path.display().to_string(),
        &questions,
        responses,
    ))
}

async fn retrieve_for_evaluation(
    service: &queria_search::retrieval::PgRetrievalService,
    user_id: uuid::Uuid,
    request: RetrieveContextRequest,
) -> Result<RetrieveContextResponse, String> {
    for attempt in 0..EVALUATION_RETRY_ATTEMPTS {
        let response = service
            .retrieve_context(&RetrievalPrincipal::User { user_id }, request.clone())
            .await
            .map_err(|error| error.to_string())?;
        if should_retry_evaluation_response(&response) && attempt + 1 < EVALUATION_RETRY_ATTEMPTS {
            tracing::warn!(
                attempt = attempt + 1,
                project_id = %request.project_id,
                "empty semantic fallback during evaluation; retrying"
            );
            tokio::time::sleep(Duration::from_millis(EVALUATION_RETRY_DELAY_MS)).await;
            continue;
        }
        return Ok(response);
    }

    Err("evaluation retry attempts exhausted".to_owned())
}

fn should_retry_evaluation_response(response: &RetrieveContextResponse) -> bool {
    response.retrieval.mode == RetrievalMode::LexicalFallback && response.items.is_empty()
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
    use queria_core::contracts::{
        RetrievalDiagnostics, RetrievalMode, RetrieveContextResponse, RetrievedContextItem,
    };
    use queria_core::ids::ProjectId;

    #[test]
    fn evaluation_retry_only_targets_empty_lexical_fallback() {
        assert!(should_retry_evaluation_response(&response(
            RetrievalMode::LexicalFallback,
            Vec::new()
        )));
        assert!(!should_retry_evaluation_response(&response(
            RetrievalMode::Hybrid,
            Vec::new()
        )));
    }

    fn response(mode: RetrievalMode, items: Vec<RetrievedContextItem>) -> RetrieveContextResponse {
        RetrieveContextResponse {
            project_id: ProjectId::new(),
            query: "query".to_owned(),
            items,
            retrieval: RetrievalDiagnostics {
                mode,
                lexical_candidates: 0,
                semantic_candidates: 0,
                embedding_profile_version: "voyage-4-1024-v1".to_owned(),
            },
            generated_at: chrono::Utc::now(),
        }
    }
}
