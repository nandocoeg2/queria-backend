use crate::manifest::{BackupManifest, sha256_hex};
use crate::object_store::ObjectStore;
use queria_core::{QueriaError, QueriaResult};
use std::process::Stdio;
use tokio::process::Command;

/// Result of a restore drill verification.
#[derive(Clone, Debug)]
pub struct DrillReport {
    pub manifest_found: bool,
    pub manifest_signature_ok: bool,
    pub pg_dump_exists: bool,
    pub pg_dump_checksum_ok: bool,
    pub qdrant_snapshot_exists: bool,
    pub qdrant_snapshot_checksum_ok: bool,
    pub all_passed: bool,
    pub errors: Vec<String>,
}

impl DrillReport {
    fn ready_to_restore(&self) -> bool {
        self.manifest_found
            && self.manifest_signature_ok
            && self.pg_dump_exists
            && self.pg_dump_checksum_ok
            && self.qdrant_snapshot_exists
            && self.qdrant_snapshot_checksum_ok
            && self.errors.is_empty()
    }
}

/// Optional destructive restore targets for a restore drill.
#[derive(Clone, Debug, Default)]
pub struct RestoreDrillOptions {
    pub manifest_signing_key: String,
    pub target_database_url: Option<String>,
    pub target_qdrant_url: Option<String>,
    pub target_qdrant_api_key: String,
    pub target_qdrant_collection: Option<String>,
}

impl RestoreDrillOptions {
    pub fn should_restore_postgres(&self) -> bool {
        self.target_database_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }

    pub fn should_restore_qdrant(&self) -> bool {
        self.target_qdrant_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
            && self
                .target_qdrant_collection
                .as_deref()
                .is_some_and(|collection| !collection.trim().is_empty())
    }
}

/// Execute a restore drill: download the latest manifest and verify all artifact
/// checksums without actually restoring anything.
///
/// This is a non-destructive read-only verification.
pub async fn run_restore_drill(
    store: &ObjectStore,
    org_slug: &str,
    manifest_signing_key: &str,
) -> QueriaResult<DrillReport> {
    run_restore_drill_with_options(
        store,
        org_slug,
        RestoreDrillOptions {
            manifest_signing_key: manifest_signing_key.to_owned(),
            ..RestoreDrillOptions::default()
        },
    )
    .await
}

/// Execute a restore drill and optionally run `pg_restore` into an empty target database.
pub async fn run_restore_drill_with_options(
    store: &ObjectStore,
    org_slug: &str,
    options: RestoreDrillOptions,
) -> QueriaResult<DrillReport> {
    let mut report = DrillReport {
        manifest_found: false,
        manifest_signature_ok: false,
        pg_dump_exists: false,
        pg_dump_checksum_ok: false,
        qdrant_snapshot_exists: false,
        qdrant_snapshot_checksum_ok: false,
        all_passed: false,
        errors: Vec::new(),
    };
    let mut pg_dump_data = None;
    let mut qdrant_snapshot_data = None;

    // Step 1: Find the latest manifest
    let prefix = format!("{org_slug}/manifests/");
    let objects = store.list_objects(&prefix).await?;

    let latest_manifest_key = objects
        .iter()
        .filter(|obj| obj.key.ends_with(".json"))
        .max_by(|a, b| a.key.cmp(&b.key))
        .map(|obj| obj.key.clone());

    let Some(manifest_key) = latest_manifest_key else {
        report.errors.push("no manifest found in S3".to_owned());
        return Ok(report);
    };

    tracing::info!(manifest = %manifest_key, "starting restore drill");

    // Step 2: Download and parse manifest
    let manifest_data = store.get_object(&manifest_key).await?;
    let manifest = BackupManifest::from_json_bytes(&manifest_data)
        .map_err(|error| QueriaError::Infrastructure(format!("manifest parse failed: {error}")))?;
    report.manifest_found = true;
    if !manifest.verify_signature(&options.manifest_signing_key) {
        report
            .errors
            .push("manifest signature missing or invalid".to_owned());
        return Ok(report);
    }
    report.manifest_signature_ok = true;

    // Step 3: Verify PostgreSQL dump
    if manifest.pg_dump_key.is_empty() {
        report
            .errors
            .push("manifest has empty pg_dump_key".to_owned());
    } else {
        match store.get_object(&manifest.pg_dump_key).await {
            Ok(data) => {
                report.pg_dump_exists = true;
                let checksum = sha256_hex(&data);
                if manifest.verify_checksum(&manifest.pg_dump_key, &data) {
                    report.pg_dump_checksum_ok = true;
                    pg_dump_data = Some(data);
                    tracing::info!(
                        key = %manifest.pg_dump_key,
                        checksum,
                        "pg_dump checksum verified"
                    );
                } else {
                    let expected = manifest
                        .checksums
                        .get(&manifest.pg_dump_key)
                        .cloned()
                        .unwrap_or_default();
                    report.errors.push(format!(
                        "pg_dump checksum mismatch: expected={expected}, actual={checksum}"
                    ));
                }
            }
            Err(error) => {
                report
                    .errors
                    .push(format!("pg_dump download failed: {error}"));
            }
        }
    }

    // Step 4: Verify Qdrant snapshot (if present)
    if let Some(ref qdrant_key) = manifest.qdrant_snapshot_key {
        match store.get_object(qdrant_key).await {
            Ok(data) => {
                report.qdrant_snapshot_exists = true;
                let checksum = sha256_hex(&data);
                if manifest.verify_checksum(qdrant_key, &data) {
                    report.qdrant_snapshot_checksum_ok = true;
                    qdrant_snapshot_data = Some(data);
                    tracing::info!(
                        key = %qdrant_key,
                        checksum,
                        "Qdrant snapshot checksum verified"
                    );
                } else {
                    let expected = manifest
                        .checksums
                        .get(qdrant_key)
                        .cloned()
                        .unwrap_or_default();
                    report.errors.push(format!(
                        "Qdrant snapshot checksum mismatch: expected={expected}, actual={checksum}"
                    ));
                }
            }
            Err(error) => {
                report
                    .errors
                    .push(format!("Qdrant snapshot download failed: {error}"));
            }
        }
    } else {
        report
            .errors
            .push("manifest has no qdrant_snapshot_key".to_owned());
    }

    // Step 5: Restore only after every artifact has passed verification.
    if report.ready_to_restore() {
        if let (Some(data), Some(target_database_url)) = (
            pg_dump_data.as_deref(),
            options.target_database_url.as_deref(),
        ) {
            if let Err(error) = restore_postgres_dump(data, target_database_url).await {
                report.errors.push(format!("pg_restore failed: {error}"));
            }
        }
        if report.errors.is_empty() && options.should_restore_qdrant() {
            let qdrant_url = options.target_qdrant_url.as_deref().unwrap_or_default();
            let collection = options
                .target_qdrant_collection
                .as_deref()
                .unwrap_or_default();
            if let Err(error) = restore_qdrant_snapshot(
                qdrant_snapshot_data.as_deref().unwrap_or_default(),
                qdrant_url,
                &options.target_qdrant_api_key,
                collection,
            )
            .await
            {
                report
                    .errors
                    .push(format!("Qdrant restore failed: {error}"));
            }
        }
    }

    report.all_passed = report.ready_to_restore();

    if report.all_passed {
        tracing::info!("restore drill PASSED");
    } else {
        tracing::warn!(errors = ?report.errors, "restore drill FAILED");
    }

    Ok(report)
}

async fn restore_qdrant_snapshot(
    data: &[u8],
    qdrant_url: &str,
    qdrant_api_key: &str,
    collection: &str,
) -> QueriaResult<()> {
    let (content_type, body) = multipart_snapshot_body(data);
    let url = format!(
        "{}/collections/{collection}/snapshots/upload?wait=true&priority=snapshot",
        qdrant_url.trim_end_matches('/')
    );
    let mut request = reqwest::Client::new()
        .post(url)
        .header("content-type", content_type)
        .body(body);
    if !qdrant_api_key.is_empty() {
        request = request.header("api-key", qdrant_api_key);
    }
    let response = request.send().await.map_err(|error| {
        QueriaError::Infrastructure(format!("Qdrant snapshot upload failed: {error}"))
    })?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(QueriaError::Infrastructure(format!(
            "Qdrant snapshot upload returned {status}: {body}"
        )))
    }
}

fn multipart_snapshot_body(data: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("queria-{}", uuid::Uuid::now_v7());
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"snapshot\"; filename=\"restore.snapshot\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn restore_postgres_dump(data: &[u8], target_database_url: &str) -> QueriaResult<()> {
    let path = std::env::temp_dir().join(format!("queria-restore-{}.dump", uuid::Uuid::now_v7()));
    std::fs::write(&path, data).map_err(|error| {
        QueriaError::Infrastructure(format!("failed to write temporary restore dump: {error}"))
    })?;

    let output = Command::new("pg_restore")
        .arg("--clean")
        .arg("--if-exists")
        .arg("--no-owner")
        .arg("--no-privileges")
        .arg("--dbname")
        .arg(target_database_url)
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| {
            QueriaError::Infrastructure(format!("failed to execute pg_restore: {error}"))
        });

    let _ = std::fs::remove_file(&path);

    let output = output?;
    if output.status.success() {
        Ok(())
    } else {
        Err(QueriaError::Infrastructure(format!(
            "pg_restore exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drill_report_default_is_failed() {
        let report = DrillReport {
            manifest_found: false,
            manifest_signature_ok: false,
            pg_dump_exists: false,
            pg_dump_checksum_ok: false,
            qdrant_snapshot_exists: false,
            qdrant_snapshot_checksum_ok: false,
            all_passed: false,
            errors: vec!["test".to_owned()],
        };
        assert!(!report.all_passed);
        assert!(!report.ready_to_restore());
    }

    #[test]
    fn restore_drill_options_default_to_read_only() {
        let options = RestoreDrillOptions {
            manifest_signing_key: "secret".to_owned(),
            ..RestoreDrillOptions::default()
        };

        assert!(!options.should_restore_postgres());
    }

    #[test]
    fn restore_drill_options_enable_postgres_restore_when_target_is_set() {
        let options = RestoreDrillOptions {
            manifest_signing_key: "secret".to_owned(),
            target_database_url: Some(
                "postgres://queria:queria@127.0.0.1:17675/restore".to_owned(),
            ),
            ..RestoreDrillOptions::default()
        };

        assert!(options.should_restore_postgres());
    }

    #[test]
    fn restore_drill_options_enable_qdrant_restore_when_target_is_complete() {
        let options = RestoreDrillOptions {
            manifest_signing_key: "secret".to_owned(),
            target_qdrant_url: Some("http://127.0.0.1:17676".to_owned()),
            target_qdrant_collection: Some("queria_restore".to_owned()),
            ..RestoreDrillOptions::default()
        };

        assert!(options.should_restore_qdrant());
    }

    #[test]
    fn qdrant_snapshot_multipart_contains_snapshot_bytes() {
        let (content_type, body) = multipart_snapshot_body(b"snapshot-data");

        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(
            body.windows(b"snapshot-data".len())
                .any(|window| window == b"snapshot-data")
        );
    }
}
