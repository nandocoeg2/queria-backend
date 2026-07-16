mod backup_jobs;
mod embedding_jobs;
mod jobs;

use queria_backup::object_store::ObjectStore;
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
    queria_core::init_json_tracing("queria-worker", &config.log_level);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await?;
    queria_db::migrate::run_migrations(&pool).await?;
    let repository = PgIngestionRepository::new(pool.clone());
    let embedding_repository = PgEmbeddingRepository::new(pool.clone());
    let recovered = repository
        .recover_expired_leases(i64::try_from(config.worker.lease_seconds)?)
        .await?;
    if recovered > 0 {
        tracing::warn!(
            recovered_jobs = recovered,
            "recovered expired ingestion leases"
        );
    }
    let recovered_embedding = embedding_repository
        .recover_expired_leases(i64::try_from(config.worker.lease_seconds)?)
        .await?;
    if recovered_embedding > 0 {
        tracing::warn!(
            recovered_jobs = recovered_embedding,
            "recovered expired embedding leases"
        );
    }

    let policy = GitSecurityPolicy::new(
        config.git.allowed_roots.iter().map(PathBuf::from).collect(),
        config.git.allowed_ssh_hosts.clone(),
        config.git.allowed_ssh_repositories.clone(),
        config.git.excluded_directories.clone(),
        config.git.max_file_bytes,
    )?;
    let service = GitIngestionService::new(
        GitCliGateway::new(policy.clone()),
        TruffleHogScanner::new(
            config.git.trufflehog_executable.clone(),
            PathBuf::from(&config.git.trufflehog_include_paths_file),
            PathBuf::from(&config.git.trufflehog_exclude_paths_file),
            Duration::from_secs(config.git.trufflehog_timeout_seconds),
        ),
        policy,
        usize::try_from(config.git.chunk_max_lines)?,
        usize::try_from(config.git.chunk_overlap_lines)?,
    );
    let poll_interval = Duration::from_millis(config.worker.poll_interval_ms);
    let embedding_config = embedding_jobs::EmbeddingWorkerConfig {
        provider: "voyage".to_owned(),
        model: config.embedding.model.clone(),
        dimension: usize::try_from(config.embedding.dimension)?,
        profile_version: config.embedding.profile_version.clone(),
        batch_size: i64::from(config.embedding.batch_size),
        request_interval_ms: config.embedding.request_interval_ms,
        retry_backoff_base_seconds: i64::try_from(config.embedding.retry_backoff_base_seconds)?,
        retry_backoff_max_seconds: i64::try_from(config.embedding.retry_backoff_max_seconds)?,
    };
    let voyage = VoyageClient::new(
        config.embedding.voyage_api_key.clone(),
        config.embedding.model.clone(),
        usize::try_from(config.embedding.dimension)?,
        Duration::from_secs(config.embedding.timeout_seconds),
        config.embedding.max_retries,
    )?;
    let qdrant = QdrantClient::new(QdrantConfig {
        url: config.qdrant.url.clone(),
        api_key: config.qdrant.api_key.clone(),
        collection: config.qdrant.collection.clone(),
        vector_name: config.qdrant.vector_name.clone(),
        dimension: usize::try_from(config.embedding.dimension)?,
    })?;
    qdrant.ensure_collection().await?;

    // Initialize object store for backups
    let object_store = ObjectStore::new(
        &config.minio.endpoint,
        &config.minio.bucket,
        &config.minio.access_key,
        &config.minio.secret_key,
        &config.minio.region,
    )?;
    object_store.ensure_bucket().await.unwrap_or_else(|error| {
        tracing::warn!(error = %error, "failed to ensure S3 bucket (will retry on backup)");
    });

    tracing::info!(
        worker_id = %config.worker.identity,
        embedding_profile = %config.embedding.profile_version,
        qdrant_collection = %config.qdrant.collection,
        embedding_request_interval_ms = config.embedding.request_interval_ms,
        backup_cron_hour_utc = config.backup.cron_hour_utc,
        backup_retention_days = config.backup.retention_days,
        "ingestion, embedding, and backup worker started"
    );

    loop {
        match jobs::run_one(&repository, &service, &config.worker.identity).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => tracing::error!(error = %error, "ingestion worker iteration failed"),
        }
        match embedding_jobs::run_one(
            &embedding_repository,
            &voyage,
            &qdrant,
            &embedding_config,
            &config.worker.identity,
        )
        .await
        {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => tracing::error!(error = %error, "embedding worker iteration failed"),
        }

        // Check if it's time for a scheduled backup
        if backup_jobs::should_run_backup(&pool, config.backup.cron_hour_utc).await {
            if let Err(error) = backup_jobs::run_backup(&object_store, &pool, &config).await {
                tracing::error!(error = %error, "scheduled backup failed");
            }
        }

        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                tracing::info!("ingestion, embedding, and backup worker stopping");
                break;
            }
            () = tokio::time::sleep(poll_interval) => {}
        }
    }
    Ok(())
}
