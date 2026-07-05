use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// A signed backup manifest that accompanies every backup set in object storage.
///
/// The manifest records exactly what was backed up, the schema version at the time
/// of backup, embedding profile, and SHA-256 checksums for every artifact so that
/// restore drills can verify integrity without downloading full files.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Manifest format version (currently "1").
    pub version: String,
    /// When the backup was created.
    pub created_at: DateTime<Utc>,
    /// Organization slug.
    pub org_slug: String,
    /// Latest migration version applied at backup time.
    pub schema_version: String,
    /// Embedding profile version (e.g. "voyage-4-1024-v1").
    pub embedding_profile: String,
    /// S3 key for the PostgreSQL dump artifact.
    pub pg_dump_key: String,
    /// S3 key for the Qdrant snapshot artifact (if taken).
    pub qdrant_snapshot_key: Option<String>,
    /// Git/source commit associated with the backed-up data and binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    /// Map of artifact S3 key → SHA-256 hex digest.
    pub checksums: BTreeMap<String, String>,
    /// `pg_dump --version` output for reproducibility.
    pub pg_dump_version: String,
    /// HMAC-SHA256 signature over manifest metadata and checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl BackupManifest {
    /// Create a new manifest with the current UTC timestamp.
    pub fn new(org_slug: &str, schema_version: &str, embedding_profile: &str) -> Self {
        Self {
            version: "1".to_owned(),
            created_at: Utc::now(),
            org_slug: org_slug.to_owned(),
            schema_version: schema_version.to_owned(),
            embedding_profile: embedding_profile.to_owned(),
            pg_dump_key: String::new(),
            qdrant_snapshot_key: None,
            source_commit: None,
            checksums: BTreeMap::new(),
            pg_dump_version: String::new(),
            signature: None,
        }
    }

    /// Record a checksum for an artifact key.
    pub fn add_checksum(&mut self, key: &str, sha256_hex: &str) {
        self.checksums.insert(key.to_owned(), sha256_hex.to_owned());
    }

    /// Serialize the manifest to pretty JSON bytes.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("manifest serialization cannot fail")
    }

    /// Deserialize a manifest from JSON bytes.
    pub fn from_json_bytes(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Verify that a given artifact's data matches the recorded checksum.
    pub fn verify_checksum(&self, key: &str, data: &[u8]) -> bool {
        let Some(expected) = self.checksums.get(key) else {
            return false;
        };
        let actual = sha256_hex(data);
        *expected == actual
    }

    /// Sign the manifest with a deployment secret.
    pub fn sign(&mut self, secret: &str) {
        self.signature = Some(self.compute_signature(secret));
    }

    /// Verify the manifest signature.
    pub fn verify_signature(&self, secret: &str) -> bool {
        let expected = self.compute_signature(secret);
        self.signature.as_deref().is_some_and(|signature| {
            signature.len() == expected.len()
                && signature
                    .bytes()
                    .zip(expected.bytes())
                    .fold(0, |difference, (left, right)| difference | (left ^ right))
                    == 0
        })
    }

    fn compute_signature(&self, secret: &str) -> String {
        let mut payload = Vec::new();
        for value in [
            self.version.as_str(),
            &self.created_at.to_rfc3339(),
            &self.org_slug,
            &self.schema_version,
            &self.embedding_profile,
            &self.pg_dump_key,
        ] {
            payload.extend_from_slice(value.as_bytes());
            payload.push(0);
        }
        if let Some(qdrant_key) = &self.qdrant_snapshot_key {
            payload.extend_from_slice(qdrant_key.as_bytes());
        }
        payload.push(0);
        if let Some(source_commit) = &self.source_commit {
            payload.extend_from_slice(source_commit.as_bytes());
        }
        payload.push(0);
        payload.extend_from_slice(self.pg_dump_version.as_bytes());
        for (key, checksum) in &self.checksums {
            payload.push(0);
            payload.extend_from_slice(key.as_bytes());
            payload.push(b'=');
            payload.extend_from_slice(checksum.as_bytes());
        }
        hmac_sha256_hex(secret.as_bytes(), &payload)
    }
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut normalized_key = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        inner_pad[index] ^= normalized_key[index];
        outer_pad[index] ^= normalized_key[index];
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(data);
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    format!("{:x}", outer.finalize())
}

/// Compute the SHA-256 hex digest of some data.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Build the S3 object key prefix for a given org and artifact type.
pub fn artifact_prefix(org_slug: &str, artifact_type: &str) -> String {
    let today = Utc::now().format("%Y-%m-%d");
    format!("{org_slug}/{artifact_type}/{today}")
}

/// Build a full artifact key including the filename.
pub fn artifact_key(org_slug: &str, artifact_type: &str, filename: &str) -> String {
    let prefix = artifact_prefix(org_slug, artifact_type);
    format!("{prefix}/{filename}")
}

/// Build the manifest key for today's backup.
pub fn manifest_key(org_slug: &str) -> String {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    artifact_key(org_slug, "manifests", &format!("manifest_{timestamp}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip() {
        let mut manifest = BackupManifest::new("fjulian", "20260705000100", "voyage-4-1024-v1");
        manifest.pg_dump_key = "fjulian/pg-dump/2026-07-05/queria.dump".to_owned();
        manifest.source_commit = Some("abc123".to_owned());
        manifest.add_checksum(&manifest.pg_dump_key.clone(), "abc123def456");

        let json = manifest.to_json_bytes();
        let restored = BackupManifest::from_json_bytes(&json).unwrap();

        assert_eq!(restored.version, "1");
        assert_eq!(restored.org_slug, "fjulian");
        assert_eq!(restored.schema_version, "20260705000100");
        assert_eq!(restored.embedding_profile, "voyage-4-1024-v1");
        assert_eq!(restored.source_commit, Some("abc123".to_owned()));
        assert_eq!(
            restored
                .checksums
                .get("fjulian/pg-dump/2026-07-05/queria.dump"),
            Some(&"abc123def456".to_owned())
        );
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let data = b"hello world";
        let hash1 = sha256_hex(data);
        let hash2 = sha256_hex(data);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn verify_checksum_detects_mismatch() {
        let mut manifest = BackupManifest::new("test", "v1", "p1");
        manifest.add_checksum("file.dump", &sha256_hex(b"correct data"));

        assert!(manifest.verify_checksum("file.dump", b"correct data"));
        assert!(!manifest.verify_checksum("file.dump", b"wrong data"));
        assert!(!manifest.verify_checksum("missing.dump", b"any"));
    }

    #[test]
    fn signed_manifest_detects_tampering() {
        let mut manifest = BackupManifest::new("test", "v1", "p1");
        manifest.pg_dump_key = "test/pg-dump/2026-07-05/db.dump".to_owned();
        manifest.qdrant_snapshot_key =
            Some("test/qdrant-snapshot/2026-07-05/q.snapshot".to_owned());
        manifest.add_checksum("test/pg-dump/2026-07-05/db.dump", "pg-checksum");
        manifest.add_checksum("test/qdrant-snapshot/2026-07-05/q.snapshot", "q-checksum");

        manifest.sign("secret");

        assert!(manifest.verify_signature("secret"));

        manifest.schema_version = "tampered".to_owned();
        assert!(!manifest.verify_signature("secret"));
    }

    #[test]
    fn manifest_signature_uses_hmac_sha256() {
        assert_eq!(
            hmac_sha256_hex(b"key", b"The quick brown fox jumps over the lazy dog"),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn artifact_key_partitions_by_org_type_date() {
        let key = artifact_key("fjulian", "pg-dump", "queria.dump");
        assert!(key.starts_with("fjulian/pg-dump/"));
        assert!(key.ends_with("/queria.dump"));
    }
}
