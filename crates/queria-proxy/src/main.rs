mod health;
mod routes;

use queria_core::AppConfig;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    queria_observability::init_json_tracing("queria-proxy", "info");
    let config = AppConfig::from_env()?;
    let app = routes::build_router();
    let listener = tokio::net::TcpListener::bind(config.proxy_addr.parse::<SocketAddr>()?).await?;
    tracing::info!(addr = %config.proxy_addr, "queria-proxy listening");
    axum::serve(listener, app).await?;
    Ok(())
}
