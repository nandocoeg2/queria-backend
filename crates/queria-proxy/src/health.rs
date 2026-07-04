use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ProxyHealth {
    pub status: &'static str,
    pub service: &'static str,
}

pub async fn healthz() -> Json<ProxyHealth> {
    Json(ProxyHealth {
        status: "ok",
        service: "queria-proxy",
    })
}
