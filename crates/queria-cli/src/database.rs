use queria_core::AppConfig;

pub async fn migrate() -> anyhow::Result<()> {
    let config = AppConfig::from_env()?;
    let pool = queria_db::pool::connect(&config.database_url).await?;
    queria_db::migrate::run_migrations(&pool).await?;
    println!(
        "{}",
        serde_json::json!({
            "status": "migrated"
        })
    );
    Ok(())
}
