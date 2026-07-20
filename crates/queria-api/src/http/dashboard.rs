use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct IngestionJobSummary {
    id: String,
    status: String,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    error_message: Option<String>,
}

#[derive(Debug, Serialize)]
struct EvaluationSummary {
    id: String,
    project_slug: String,
    score: f32,
    passed: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ChunkCountsByEmbeddingState {
    pending: i64,
    processing: i64,
    ready: i64,
    failed: i64,
    stale: i64,
}

#[derive(Debug, Serialize)]
struct DashboardSummaryResponse {
    project_count: i64,
    source_count: i64,
    pending_approvals_count: i64,
    agent_token_count: i64,
    chunk_counts: ChunkCountsByEmbeddingState,
    failed_jobs_count: i64,
    latest_ingestion: Option<IngestionJobSummary>,
    latest_evaluation: Option<EvaluationSummary>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new().route("/summary", get(get_summary))
}

async fn get_summary(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<DashboardSummaryResponse> {
    let session = auth::require_session(&state, &headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let Some(repository) = state.admin_queries_repository() else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "admin_queries_store_not_configured",
        ));
    };

    let summary = repository
        .get_dashboard_summary(session.user_id)
        .await
        .map_err(map_error)?;

    Ok(Json(DashboardSummaryResponse::from(summary)))
}

impl From<queria_db::admin_queries::DashboardSummaryRecord> for DashboardSummaryResponse {
    fn from(value: queria_db::admin_queries::DashboardSummaryRecord) -> Self {
        let latest_ingestion = value.latest_ingestion_id.map(|id| IngestionJobSummary {
            id: id.to_string(),
            status: value.latest_ingestion_status.unwrap_or_default(),
            started_at: value.latest_ingestion_started_at,
            finished_at: value.latest_ingestion_finished_at,
            error_message: value.latest_ingestion_error_message,
        });

        let latest_evaluation = value.latest_evaluation_id.map(|id| EvaluationSummary {
            id: id.to_string(),
            project_slug: value.latest_evaluation_project_slug.unwrap_or_default(),
            score: value.latest_evaluation_score.unwrap_or(0.0) as f32,
            passed: value.latest_evaluation_passed.unwrap_or(false),
            created_at: value.latest_evaluation_created_at.unwrap_or_else(Utc::now),
        });

        Self {
            project_count: value.project_count,
            source_count: value.source_count,
            pending_approvals_count: value.pending_approvals_count,
            agent_token_count: value.agent_token_count,
            chunk_counts: ChunkCountsByEmbeddingState {
                pending: value.chunks_pending,
                processing: value.chunks_processing,
                ready: value.chunks_ready,
                failed: value.chunks_failed,
                stale: value.chunks_stale,
            },
            failed_jobs_count: value.failed_jobs_count,
            latest_ingestion,
            latest_evaluation,
        }
    }
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "dashboard repository failed");
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
