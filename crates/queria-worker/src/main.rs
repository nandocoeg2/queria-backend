mod embedding_jobs;
mod jobs;

use queria_core::AppConfig;
use queria_db::embedding::PgEmbeddingRepository;
use queria_db::ingestion::PgIngestionRepository;
use queria_ingestion::git::{GitCliGateway, GitSecurityPolicy};
use queria_ingestion::scanner::TruffleHogScanner;
use queria_ingestion::service::GitIngestionService;
use queria_search::embedding::VectorIndex;
use queria_search::qdrant::{QdrantClient, QdrantConfig};
use queria_search::voyage::VoyageClient;
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
    let repository = PgIngestionRepository::new(pool.clone());
    let embedding_repository = PgEmbeddingRepository::new(pool);
    let recovered = repository
        .recover_expired_leases(i64::try_from(config.worker_lease_seconds)?)
        .await?;
    if recovered > 0 {
        tracing::warn!(
            recovered_jobs = recovered,
            "recovered expired ingestion leases"
        );
    }
    let recovered_embedding = embedding_repository
        .recover_expired_leases(i64::try_from(config.worker_lease_seconds)?)
        .await?;
    if recovered_embedding > 0 {
        tracing::warn!(
            recovered_jobs = recovered_embedding,
            "recovered expired embedding leases"
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
    let embedding_config = embedding_jobs::EmbeddingWorkerConfig {
        provider: "voyage".to_owned(),
        model: config.embedding_model.clone(),
        dimension: usize::try_from(config.embedding_dimension)?,
        profile_version: config.embedding_profile_version.clone(),
        batch_size: i64::from(config.embedding_batch_size),
        request_interval_ms: config.embedding_request_interval_ms,
        retry_backoff_base_seconds: i64::try_from(config.embedding_retry_backoff_base_seconds)?,
        retry_backoff_max_seconds: i64::try_from(config.embedding_retry_backoff_max_seconds)?,
    };
    let voyage = VoyageClient::new(
        config.voyage_api_key.clone(),
        config.embedding_model.clone(),
        usize::try_from(config.embedding_dimension)?,
        Duration::from_secs(config.embedding_timeout_seconds),
        config.embedding_max_retries,
    )?;
    let qdrant = QdrantClient::new(QdrantConfig {
        url: config.qdrant_url.clone(),
        api_key: config.qdrant_api_key.clone(),
        collection: config.qdrant_collection.clone(),
        vector_name: config.qdrant_vector_name.clone(),
        dimension: usize::try_from(config.embedding_dimension)?,
    })?;
    qdrant.ensure_collection().await?;
    tracing::info!(
        worker_id = %config.worker_identity,
        embedding_profile = %config.embedding_profile_version,
        qdrant_collection = %config.qdrant_collection,
        embedding_request_interval_ms = config.embedding_request_interval_ms,
        "ingestion and embedding worker started"
    );

    loop {
        match jobs::run_one(&repository, &service, &config.worker_identity).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => tracing::error!(error = %error, "ingestion worker iteration failed"),
        }
        match embedding_jobs::run_one(
            &embedding_repository,
            &voyage,
            &qdrant,
            &embedding_config,
            &config.worker_identity,
        )
        .await
        {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => tracing::error!(error = %error, "embedding worker iteration failed"),
        }
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                tracing::info!("ingestion and embedding worker stopping");
                break;
            }
            () = tokio::time::sleep(poll_interval) => {}
        }
    }
    Ok(())
}
