use crate::manifest::{artifact_key, sha256_hex};
use crate::object_store::ObjectStore;
use queria_core::{QueriaError, QueriaResult};
use std::process::Stdio;
use tokio::process::Command;

/// Run `pg_dump` against the given database URL and upload the result to object storage.
///
/// Returns `(s3_key, sha256_hex, size_bytes)` on success.
pub async fn backup_postgres(
    store: &ObjectStore,
    database_url: &str,
    org_slug: &str,
) -> QueriaResult<(String, String, u64)> {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let filename = format!("queria_{timestamp}.dump");
    let key = artifact_key(org_slug, "pg-dump", &filename);

    tracing::info!(key = %key, "starting PostgreSQL backup");

    let output = Command::new("pg_dump")
        .arg("--format=custom")
        .arg("--no-owner")
        .arg("--no-privileges")
        .arg("--dbname")
        .arg(database_url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| {
            QueriaError::Infrastructure(format!(
                "failed to execute pg_dump: {error}. Is postgresql-client installed?"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(QueriaError::Infrastructure(format!(
            "pg_dump exited with {}: {stderr}",
            output.status
        )));
    }

    let data = &output.stdout;
    let checksum = sha256_hex(data);
    let size = u64::try_from(data.len()).unwrap_or(0);

    store
        .put_object(&key, data, "application/octet-stream")
        .await?;

    tracing::info!(
        key = %key,
        size_bytes = size,
        checksum = %checksum,
        "PostgreSQL backup uploaded"
    );

    Ok((key, checksum, size))
}

/// Get the `pg_dump --version` output string.
pub async fn pg_dump_version() -> QueriaResult<String> {
    let output = Command::new("pg_dump")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| {
            QueriaError::Infrastructure(format!("pg_dump --version failed: {error}"))
        })?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_key_contains_org_and_pg_dump() {
        let key = artifact_key("fjulian", "pg-dump", "queria_test.dump");
        assert!(key.contains("fjulian/pg-dump/"));
        assert!(key.ends_with("/queria_test.dump"));
    }
}
