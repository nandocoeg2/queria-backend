use axum::{Json, Router, routing::post};
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/retrieve-context", post(retrieve_context))
}

async fn retrieve_context(
    Json(request): Json<RetrieveContextRequest>,
) -> Json<RetrieveContextResponse> {
    let response = RetrieveContextResponse {
        project_id: request.project_id,
        query: request.query,
        items: Vec::new(),
        generated_at: chrono::Utc::now(),
    };
    Json(response)
}
