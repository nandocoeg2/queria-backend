use queria_core::AppConfig;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    queria_observability::init_json_tracing("queria-api", "info");
    let config = AppConfig::from_env()?;
    let app = queria_api::app::build_app(config.clone());
    let listener = tokio::net::TcpListener::bind(config.api_addr.parse::<SocketAddr>()?).await?;
    tracing::info!(addr = %config.api_addr, "queria-api listening");
    axum::serve(listener, app).await?;
    Ok(())
}
