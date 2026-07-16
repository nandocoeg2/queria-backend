use queria_core::AppConfig;
use queria_db::repositories::PgProjectRepository;
use queria_db::{migrate, pool};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    queria_core::init_json_tracing("queria-api", "info");
    let config = AppConfig::from_env()?;
    let db_pool = pool::connect(&config.database_url).await?;
    migrate::run_migrations(&db_pool).await?;
    PgProjectRepository::new(db_pool.clone())
        .seed_fjulian_me_registry()
        .await?;
    let app = queria_api::app::build_app_with_pool(config.clone(), db_pool);
    let listener = tokio::net::TcpListener::bind(config.api_addr.parse::<SocketAddr>()?).await?;
    tracing::info!(addr = %config.api_addr, "queria-api listening");
    axum::serve(listener, app).await?;
    Ok(())
}
