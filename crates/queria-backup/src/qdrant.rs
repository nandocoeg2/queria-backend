use crate::manifest::{artifact_key, sha256_hex};
use crate::object_store::ObjectStore;
use queria_core::{QueriaError, QueriaResult};
use reqwest::Client;
use serde::Deserialize;

/// Create a Qdrant collection snapshot, download it, and upload to S3.
///
/// Returns `(s3_key, sha256_hex, size_bytes)` on success.
pub async fn backup_qdrant(
    store: &ObjectStore,
    qdrant_url: &str,
    qdrant_api_key: &str,
    collection: &str,
    org_slug: &str,
) -> QueriaResult<(String, String, u64)> {
    let client = Client::new();
    let base = qdrant_url.trim_end_matches('/');

    tracing::info!(collection, "creating Qdrant snapshot");

    // Step 1: Create snapshot
    let create_url = format!("{base}/collections/{collection}/snapshots");
    let mut request = client.post(&create_url);
    if !qdrant_api_key.is_empty() {
        request = request.header("api-key", qdrant_api_key);
    }

    let response = request.send().await.map_err(|error| {
        QueriaError::Infrastructure(format!("Qdrant snapshot create failed: {error}"))
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(QueriaError::Infrastructure(format!(
            "Qdrant snapshot create returned {status}: {body}"
        )));
    }

    let create_response: SnapshotCreateResponse = response.json().await.map_err(|error| {
        QueriaError::Infrastructure(format!("Qdrant snapshot response parse failed: {error}"))
    })?;

    let snapshot_name = create_response.result.name;
    tracing::info!(snapshot = %snapshot_name, "Qdrant snapshot created, downloading");

    // Step 2: Download snapshot
    let download_url = format!("{base}/collections/{collection}/snapshots/{snapshot_name}");
    let mut request = client.get(&download_url);
    if !qdrant_api_key.is_empty() {
        request = request.header("api-key", qdrant_api_key);
    }

    let response = request.send().await.map_err(|error| {
        QueriaError::Infrastructure(format!("Qdrant snapshot download failed: {error}"))
    })?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(QueriaError::Infrastructure(format!(
            "Qdrant snapshot download returned {status}"
        )));
    }

    let data = response.bytes().await.map_err(|error| {
        QueriaError::Infrastructure(format!("Qdrant snapshot read failed: {error}"))
    })?;

    let checksum = sha256_hex(&data);
    let size = u64::try_from(data.len()).unwrap_or(0);

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let filename = format!("{collection}_{timestamp}.snapshot");
    let key = artifact_key(org_slug, "qdrant-snapshot", &filename);

    // Step 3: Upload to S3
    store
        .put_object(&key, &data, "application/octet-stream")
        .await?;

    tracing::info!(
        key = %key,
        size_bytes = size,
        checksum = %checksum,
        "Qdrant snapshot uploaded"
    );

    // Step 4: Delete the snapshot from Qdrant server to save disk space
    let delete_url = format!("{base}/collections/{collection}/snapshots/{snapshot_name}");
    let mut request = client.delete(&delete_url);
    if !qdrant_api_key.is_empty() {
        request = request.header("api-key", qdrant_api_key);
    }
    // Best-effort cleanup; don't fail the backup if this fails.
    if let Err(error) = request.send().await {
        tracing::warn!(error = %error, "failed to delete Qdrant server-side snapshot");
    }

    Ok((key, checksum, size))
}

#[derive(Debug, Deserialize)]
struct SnapshotCreateResponse {
    result: SnapshotInfo,
}

#[derive(Debug, Deserialize)]
struct SnapshotInfo {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_key_partitions_correctly() {
        let key = artifact_key("fjulian", "qdrant-snapshot", "test_col.snapshot");
        assert!(key.contains("fjulian/qdrant-snapshot/"));
        assert!(key.ends_with("/test_col.snapshot"));
    }
}
