use crate::tools;
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", post(mcp))
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "queria-mcp",
    })
}

async fn mcp(Json(request): Json<JsonRpcRequest>) -> Json<Value> {
    let result = match request.method.as_str() {
        "tools/list" => json!({ "tools": tools::tool_names() }),
        _ => json!({ "error": "method_not_found" }),
    };

    Json(json!({
        "jsonrpc": "2.0",
        "id": request.id,
        "result": result
    }))
}
