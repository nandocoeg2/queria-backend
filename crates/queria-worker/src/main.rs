mod jobs;

use queria_core::AppConfig;
use queria_db::ingestion::PgIngestionRepository;
use queria_ingestion::git::{GitCliGateway, GitSecurityPolicy};
use queria_ingestion::scanner::TruffleHogScanner;
use queria_ingestion::service::GitIngestionService;
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::from_env()?;
    queria_observability::init_json_tracing("queria-worker", &config.log_level);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await?;
    queria_db::migrate::run_migrations(&pool).await?;
    let repository = PgIngestionRepository::new(pool);
    let recovered = repository
        .recover_expired_leases(i64::try_from(config.worker_lease_seconds)?)
        .await?;
    if recovered > 0 {
        tracing::warn!(
            recovered_jobs = recovered,
            "recovered expired ingestion leases"
        );
    }

    let policy = GitSecurityPolicy::new(
        config.git_allowed_roots.iter().map(PathBuf::from).collect(),
        config.git_allowed_ssh_hosts.clone(),
        config.git_allowed_ssh_repositories.clone(),
        config.git_excluded_directories.clone(),
        config.git_max_file_bytes,
    )?;
    let service = GitIngestionService::new(
        GitCliGateway::new(policy.clone()),
        TruffleHogScanner::new(
            config.trufflehog_executable.clone(),
            PathBuf::from(&config.trufflehog_include_paths_file),
            PathBuf::from(&config.trufflehog_exclude_paths_file),
            Duration::from_secs(config.trufflehog_timeout_seconds),
        ),
        policy,
        usize::try_from(config.git_chunk_max_lines)?,
        usize::try_from(config.git_chunk_overlap_lines)?,
    );
    let poll_interval = Duration::from_millis(config.worker_poll_interval_ms);
    tracing::info!(worker_id = %config.worker_identity, "Git ingestion worker started");

    loop {
        match jobs::run_one(&repository, &service, &config.worker_identity).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => tracing::error!(error = %error, "ingestion worker iteration failed"),
        }
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                tracing::info!("Git ingestion worker stopping");
                break;
            }
            () = tokio::time::sleep(poll_interval) => {}
        }
    }
    Ok(())
}
