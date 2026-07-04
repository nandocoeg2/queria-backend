use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ProjectSummary {
    slug: &'static str,
    name: &'static str,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/", get(list_projects))
}

async fn list_projects() -> Json<Vec<ProjectSummary>> {
    Json(vec![ProjectSummary {
        slug: "fjulian-me",
        name: "fjulian.me",
    }])
}
