use queria_core::AppConfig;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    queria_observability::init_json_tracing("queria-mcp", "info");
    let config = AppConfig::from_env()?;
    let app = queria_mcp::server::build_app();
    let listener = tokio::net::TcpListener::bind(config.mcp_addr.parse::<SocketAddr>()?).await?;
    tracing::info!(addr = %config.mcp_addr, "queria-mcp listening");
    axum::serve(listener, app).await?;
    Ok(())
}
