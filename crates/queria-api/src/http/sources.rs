use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct SourceSummary {
    slug: &'static str,
    kind: &'static str,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/", get(list_sources))
}

async fn list_sources() -> Json<Vec<SourceSummary>> {
    Json(vec![SourceSummary {
        slug: "fjulian-me-repo",
        kind: "git_repo",
    }])
}
