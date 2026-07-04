use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct SetupStateResponse {
    setup_required: bool,
    first_admin_email: String,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/state", get(setup_state))
}

async fn setup_state() -> Json<SetupStateResponse> {
    Json(SetupStateResponse {
        setup_required: true,
        first_admin_email: "nando@fjulian.id".to_owned(),
    })
}
