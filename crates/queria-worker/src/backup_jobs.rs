use queria_backup::manifest::{BackupManifest, manifest_key};
use queria_backup::object_store::ObjectStore;
use queria_backup::postgres::{backup_postgres, pg_dump_version};
use queria_backup::qdrant::backup_qdrant;
use queria_backup::retention::run_retention;
use queria_core::AppConfig;
use sqlx::PgPool;
use std::process::Stdio;
use tokio::process::Command;

/// Check whether a backup should run right now.
///
/// Returns `true` if the current UTC hour matches `cron_hour_utc` and no
/// successful backup has been recorded today.
pub async fn should_run_backup(pool: &PgPool, cron_hour_utc: u32) -> bool {
    let now = chrono::Utc::now();
    if now.format("%H").to_string().parse::<u32>().unwrap_or(99) != cron_hour_utc {
        return false;
    }

    // Check if we already ran a successful backup today
    let today_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let already_ran: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM backup_record
            WHERE status = 'succeeded'
              AND created_at >= $1::timestamp AT TIME ZONE 'UTC'
        )",
    )
    .bind(today_start)
    .fetch_one(pool)
    .await
    .unwrap_or(true); // If query fails, skip to avoid repeated errors.

    !already_ran
}

/// Run a full backup: PostgreSQL dump + Qdrant snapshot + manifest upload.
///
/// Records the result in the `backup_record` table.
pub async fn run_backup(
    store: &ObjectStore,
    pool: &PgPool,
    config: &AppConfig,
) -> anyhow::Result<()> {
    tracing::info!("starting scheduled backup");

    // Insert a running record
    let record_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO backup_record (backup_type, status)
         VALUES ('full', 'running')
         RETURNING id",
    )
    .fetch_one(pool)
    .await?;

    let schema_version = latest_schema_version(pool).await;
    let mut manifest = BackupManifest::new(
        &config.first_org_slug,
        &schema_version,
        &config.embedding.profile_version,
    );

    // Get pg_dump version
    manifest.pg_dump_version = pg_dump_version().await.unwrap_or_default();
    manifest.source_commit = source_commit().await;

    let mut artifact_keys: Vec<String> = Vec::new();
    let mut total_size: i64 = 0;

    // 1. PostgreSQL backup
    match backup_postgres(store, &config.database_url, &config.first_org_slug).await {
        Ok((key, checksum, size)) => {
            manifest.pg_dump_key = key.clone();
            manifest.add_checksum(&key, &checksum);
            artifact_keys.push(key);
            total_size += i64::try_from(size).unwrap_or(0);
        }
        Err(error) => {
            tracing::error!(error = %error, "PostgreSQL backup failed");
            mark_failed(pool, record_id, &error.to_string()).await;
            return Err(error.into());
        }
    }

    // 2. Qdrant snapshot
    match backup_qdrant(
        store,
        &config.qdrant.url,
        &config.qdrant.api_key,
        &config.qdrant.collection,
        &config.first_org_slug,
    )
    .await
    {
        Ok((key, checksum, size)) => {
            manifest.qdrant_snapshot_key = Some(key.clone());
            manifest.add_checksum(&key, &checksum);
            artifact_keys.push(key);
            total_size += i64::try_from(size).unwrap_or(0);
        }
        Err(error) => {
            tracing::error!(error = %error, "Qdrant snapshot failed");
            mark_failed(pool, record_id, &error.to_string()).await;
            return Err(error.into());
        }
    }

    // 3. Upload manifest
    let m_key = manifest_key(&config.first_org_slug);
    manifest.sign(&config.minio.secret_key);
    let manifest_bytes = manifest.to_json_bytes();
    let manifest_checksum = queria_backup::manifest::sha256_hex(&manifest_bytes);
    total_size += i64::try_from(manifest_bytes.len()).unwrap_or(0);

    store
        .put_object(&m_key, &manifest_bytes, "application/json")
        .await?;
    artifact_keys.push(m_key.clone());

    // 4. Mark success
    let checksums_json = serde_json::to_value(&manifest.checksums)?;
    sqlx::query(
        "UPDATE backup_record
         SET status = 'succeeded',
             manifest_key = $1,
             artifact_keys = $2,
             checksums = $3,
             size_bytes = $4,
             completed_at = now()
         WHERE id = $5",
    )
    .bind(&m_key)
    .bind(&artifact_keys)
    .bind(&checksums_json)
    .bind(total_size)
    .bind(record_id)
    .execute(pool)
    .await?;

    tracing::info!(
        manifest = %m_key,
        artifacts = artifact_keys.len(),
        size_bytes = total_size,
        checksum = %manifest_checksum,
        "backup completed successfully"
    );

    // 5. Run retention cleanup
    if let Err(error) = run_retention(
        pool,
        store,
        &config.first_org_slug,
        config.backup.retention_days,
    )
    .await
    {
        tracing::warn!(error = %error, "retention cleanup failed");
    }

    Ok(())
}

async fn mark_failed(pool: &PgPool, record_id: uuid::Uuid, error_message: &str) {
    let _ = sqlx::query(
        "UPDATE backup_record
         SET status = 'failed',
             error_message = $1,
             completed_at = now()
         WHERE id = $2",
    )
    .bind(error_message)
    .bind(record_id)
    .execute(pool)
    .await;
}

async fn latest_schema_version(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT version FROM _queria_migration ORDER BY version DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "unknown".to_owned())
}

async fn source_commit() -> Option<String> {
    if let Some(commit) = std::env::var("QUERIA_SOURCE_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
    {
        return Some(commit);
    }
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|commit| !commit.is_empty())
    } else {
        None
    }
}
