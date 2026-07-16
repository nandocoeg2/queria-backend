use anyhow::{Context, bail};
use queria_backup::object_store::ObjectStore;
use queria_core::AppConfig;
use std::time::Instant;

use crate::restore_drill::{RestoreDrillOptions, run_restore_drill_with_options};

pub async fn restore_drill(
    org_slug: &str,
    target_database_url: Option<String>,
    target_qdrant_url: Option<String>,
    target_qdrant_collection: Option<String>,
) -> anyhow::Result<()> {
    let restore_requested = target_database_url.is_some()
        || target_qdrant_url.is_some()
        || target_qdrant_collection.is_some();
    if restore_requested
        && (target_database_url.is_none()
            || target_qdrant_url.is_none()
            || target_qdrant_collection.is_none())
    {
        bail!("actual restore requires all PostgreSQL and Qdrant target options");
    }

    let config = AppConfig::from_env().context("failed to load Queria configuration")?;
    let store = ObjectStore::new(
        &config.minio.endpoint,
        &config.minio.bucket,
        &config.minio.access_key,
        &config.minio.secret_key,
        &config.minio.region,
    )?;
    let started_at = Instant::now();
    let report = run_restore_drill_with_options(
        &store,
        org_slug,
        RestoreDrillOptions {
            manifest_signing_key: config.minio.secret_key,
            target_database_url,
            target_qdrant_url,
            target_qdrant_api_key: config.qdrant.api_key,
            target_qdrant_collection,
        },
    )
    .await?;

    println!(
        "restore_drill duration_ms={} report={report:?}",
        started_at.elapsed().as_millis()
    );
    if !report.all_passed {
        bail!("restore drill failed: {}", report.errors.join("; "));
    }
    Ok(())
}
