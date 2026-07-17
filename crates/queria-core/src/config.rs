use crate::{QueriaError, QueriaResult};
use serde::{Deserialize, Serialize};
use std::{env, fmt, net::SocketAddr};
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QdrantSettings {
    pub url: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub collection: String,
    pub vector_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingSettings {
    #[serde(skip_serializing)]
    pub voyage_api_key: String,
    pub model: String,
    pub dimension: u32,
    pub profile_version: String,
    pub batch_size: u32,
    pub timeout_seconds: u64,
    pub max_retries: u32,
    pub request_interval_ms: u64,
    pub retry_backoff_base_seconds: u64,
    pub retry_backoff_max_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrievalSettings {
    pub rrf_k: u32,
    pub candidate_multiplier: u32,
    pub candidate_cap: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MinioSettings {
    pub endpoint: String,
    pub bucket: String,
    #[serde(skip_serializing)]
    pub access_key: String,
    #[serde(skip_serializing)]
    pub secret_key: String,
    pub region: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupSettings {
    pub retention_days: u32,
    pub cron_hour_utc: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitSettings {
    pub allowed_roots: Vec<String>,
    pub allowed_ssh_hosts: Vec<String>,
    pub allowed_ssh_repositories: Vec<String>,
    pub excluded_directories: Vec<String>,
    pub max_file_bytes: u64,
    pub chunk_max_lines: u32,
    pub chunk_overlap_lines: u32,
    pub trufflehog_executable: String,
    pub trufflehog_include_paths_file: String,
    pub trufflehog_exclude_paths_file: String,
    pub trufflehog_timeout_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkerSettings {
    pub poll_interval_ms: u64,
    pub lease_seconds: u64,
    pub identity: String,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub environment: String,
    pub public_base_url: String,
    pub api_addr: String,
    pub mcp_addr: String,
    pub worker_health_addr: String,
    pub database_url: String,
    pub log_level: String,
    pub setup_token: String,
    pub first_admin_email: String,
    pub first_org_slug: String,
    /// Shared max UTF-8 body bytes for MCP `index_memory` and `propose_memory` (IMP-23).
    /// Env: `QUERIA_MAX_BODY_BYTES`. Default 20_000.
    pub max_body_bytes: usize,
    pub qdrant: QdrantSettings,
    pub embedding: EmbeddingSettings,
    pub retrieval: RetrievalSettings,
    pub minio: MinioSettings,
    pub backup: BackupSettings,
    pub git: GitSettings,
    pub worker: WorkerSettings,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("environment", &self.environment)
            .field("public_base_url", &self.public_base_url)
            .field("api_addr", &self.api_addr)
            .field("mcp_addr", &self.mcp_addr)
            .field("worker_health_addr", &self.worker_health_addr)
            .field("qdrant", &self.qdrant)
            .field("embedding", &self.embedding)
            .field("retrieval", &self.retrieval)
            .field("minio", &self.minio)
            .field("backup", &self.backup)
            .field("log_level", &self.log_level)
            .finish_non_exhaustive()
    }
}

impl AppConfig {
    pub fn from_env() -> QueriaResult<Self> {
        let defaults = Self::default_local();
        let config = Self {
            environment: read_env("QUERIA_ENV", &defaults.environment),
            public_base_url: read_env("QUERIA_PUBLIC_BASE_URL", &defaults.public_base_url),
            api_addr: read_env("QUERIA_API_ADDR", &defaults.api_addr),
            mcp_addr: read_env("QUERIA_MCP_ADDR", &defaults.mcp_addr),
            worker_health_addr: read_env("QUERIA_WORKER_HEALTH_ADDR", &defaults.worker_health_addr),
            database_url: read_env("QUERIA_DATABASE_URL", &defaults.database_url),
            log_level: read_env("QUERIA_LOG_LEVEL", &defaults.log_level),
            setup_token: read_env("QUERIA_SETUP_TOKEN", &defaults.setup_token),
            first_admin_email: read_env("QUERIA_FIRST_ADMIN_EMAIL", &defaults.first_admin_email),
            first_org_slug: read_env("QUERIA_FIRST_ORG_SLUG", &defaults.first_org_slug),
            max_body_bytes: read_number_env("QUERIA_MAX_BODY_BYTES", defaults.max_body_bytes)?,
            qdrant: QdrantSettings {
                url: read_env("QUERIA_QDRANT_URL", &defaults.qdrant.url),
                api_key: read_env("QDRANT_API_KEY", &defaults.qdrant.api_key),
                collection: read_env("QUERIA_QDRANT_COLLECTION", &defaults.qdrant.collection),
                vector_name: read_env("QUERIA_QDRANT_VECTOR_NAME", &defaults.qdrant.vector_name),
            },
            embedding: EmbeddingSettings {
                voyage_api_key: read_env("VOYAGE_API_KEY", &defaults.embedding.voyage_api_key),
                model: read_env("QUERIA_EMBEDDING_MODEL", &defaults.embedding.model),
                dimension: read_number_env(
                    "QUERIA_EMBEDDING_DIMENSION",
                    defaults.embedding.dimension,
                )?,
                profile_version: read_env(
                    "QUERIA_EMBEDDING_PROFILE_VERSION",
                    &defaults.embedding.profile_version,
                ),
                batch_size: read_number_env(
                    "QUERIA_EMBEDDING_BATCH_SIZE",
                    defaults.embedding.batch_size,
                )?,
                timeout_seconds: read_number_env(
                    "QUERIA_EMBEDDING_TIMEOUT_SECONDS",
                    defaults.embedding.timeout_seconds,
                )?,
                max_retries: read_number_env(
                    "QUERIA_EMBEDDING_MAX_RETRIES",
                    defaults.embedding.max_retries,
                )?,
                request_interval_ms: read_number_env(
                    "QUERIA_EMBEDDING_REQUEST_INTERVAL_MS",
                    defaults.embedding.request_interval_ms,
                )?,
                retry_backoff_base_seconds: read_number_env(
                    "QUERIA_EMBEDDING_RETRY_BACKOFF_BASE_SECONDS",
                    defaults.embedding.retry_backoff_base_seconds,
                )?,
                retry_backoff_max_seconds: read_number_env(
                    "QUERIA_EMBEDDING_RETRY_BACKOFF_MAX_SECONDS",
                    defaults.embedding.retry_backoff_max_seconds,
                )?,
            },
            retrieval: RetrievalSettings {
                rrf_k: read_number_env("QUERIA_RETRIEVAL_RRF_K", defaults.retrieval.rrf_k)?,
                candidate_multiplier: read_number_env(
                    "QUERIA_RETRIEVAL_CANDIDATE_MULTIPLIER",
                    defaults.retrieval.candidate_multiplier,
                )?,
                candidate_cap: read_number_env(
                    "QUERIA_RETRIEVAL_CANDIDATE_CAP",
                    defaults.retrieval.candidate_cap,
                )?,
            },
            minio: MinioSettings {
                endpoint: read_env("QUERIA_MINIO_ENDPOINT", &defaults.minio.endpoint),
                bucket: read_env("QUERIA_MINIO_BUCKET", &defaults.minio.bucket),
                access_key: read_env("QUERIA_MINIO_ACCESS_KEY", &defaults.minio.access_key),
                secret_key: read_env("QUERIA_MINIO_SECRET_KEY", &defaults.minio.secret_key),
                region: read_env("QUERIA_MINIO_REGION", &defaults.minio.region),
            },
            backup: BackupSettings {
                retention_days: read_number_env(
                    "QUERIA_BACKUP_RETENTION_DAYS",
                    defaults.backup.retention_days,
                )?,
                cron_hour_utc: read_number_env(
                    "QUERIA_BACKUP_CRON_HOUR_UTC",
                    defaults.backup.cron_hour_utc,
                )?,
            },
            git: GitSettings {
                allowed_roots: read_csv_env(
                    "QUERIA_GIT_ALLOWED_ROOTS",
                    &defaults.git.allowed_roots,
                ),
                allowed_ssh_hosts: read_csv_env(
                    "QUERIA_GIT_ALLOWED_SSH_HOSTS",
                    &defaults.git.allowed_ssh_hosts,
                ),
                allowed_ssh_repositories: read_csv_env(
                    "QUERIA_GIT_ALLOWED_SSH_REPOSITORIES",
                    &defaults.git.allowed_ssh_repositories,
                ),
                excluded_directories: read_csv_env(
                    "QUERIA_GIT_EXCLUDED_DIRECTORIES",
                    &defaults.git.excluded_directories,
                ),
                max_file_bytes: read_number_env(
                    "QUERIA_GIT_MAX_FILE_BYTES",
                    defaults.git.max_file_bytes,
                )?,
                chunk_max_lines: read_number_env(
                    "QUERIA_GIT_CHUNK_MAX_LINES",
                    defaults.git.chunk_max_lines,
                )?,
                chunk_overlap_lines: read_number_env(
                    "QUERIA_GIT_CHUNK_OVERLAP_LINES",
                    defaults.git.chunk_overlap_lines,
                )?,
                trufflehog_executable: read_env(
                    "QUERIA_TRUFFLEHOG_EXECUTABLE",
                    &defaults.git.trufflehog_executable,
                ),
                trufflehog_include_paths_file: read_env(
                    "QUERIA_TRUFFLEHOG_INCLUDE_PATHS_FILE",
                    &defaults.git.trufflehog_include_paths_file,
                ),
                trufflehog_exclude_paths_file: read_env(
                    "QUERIA_TRUFFLEHOG_EXCLUDE_PATHS_FILE",
                    &defaults.git.trufflehog_exclude_paths_file,
                ),
                trufflehog_timeout_seconds: read_number_env(
                    "QUERIA_TRUFFLEHOG_TIMEOUT_SECONDS",
                    defaults.git.trufflehog_timeout_seconds,
                )?,
            },
            worker: WorkerSettings {
                poll_interval_ms: read_number_env(
                    "QUERIA_WORKER_POLL_INTERVAL_MS",
                    defaults.worker.poll_interval_ms,
                )?,
                lease_seconds: read_number_env(
                    "QUERIA_WORKER_LEASE_SECONDS",
                    defaults.worker.lease_seconds,
                )?,
                identity: read_env("QUERIA_WORKER_IDENTITY", &defaults.worker.identity),
            },
        };

        config.validate()?;
        Ok(config)
    }

    pub fn default_local() -> Self {
        Self {
            environment: "local".to_owned(),
            public_base_url: "http://127.0.0.1:17674".to_owned(),
            api_addr: "127.0.0.1:17671".to_owned(),
            mcp_addr: "127.0.0.1:17672".to_owned(),
            worker_health_addr: "127.0.0.1:17673".to_owned(),
            database_url: "postgres://queria:queria@127.0.0.1:17675/queria".to_owned(),
            log_level: "info".to_owned(),
            setup_token: "change-me-one-time-setup-token".to_owned(),
            first_admin_email: "nando@fjulian.id".to_owned(),
            first_org_slug: "fjulian".to_owned(),
            max_body_bytes: 20_000,
            qdrant: QdrantSettings {
                url: "http://127.0.0.1:17676".to_owned(),
                api_key: String::new(),
                collection: "queria_local_chunks_v1".to_owned(),
                vector_name: "dense_v1".to_owned(),
            },
            embedding: EmbeddingSettings {
                voyage_api_key: String::new(),
                model: "voyage-4".to_owned(),
                dimension: 1024,
                profile_version: "voyage-4-1024-v1".to_owned(),
                batch_size: 64,
                timeout_seconds: 30,
                max_retries: 3,
                request_interval_ms: 0,
                retry_backoff_base_seconds: 30,
                retry_backoff_max_seconds: 600,
            },
            retrieval: RetrievalSettings {
                rrf_k: 60,
                candidate_multiplier: 4,
                candidate_cap: 100,
            },
            minio: MinioSettings {
                endpoint: "http://127.0.0.1:17678".to_owned(),
                bucket: "queria-local".to_owned(),
                access_key: "queria".to_owned(),
                secret_key: "queria-local-dev-only".to_owned(),
                region: "us-east-1".to_owned(),
            },
            backup: BackupSettings {
                retention_days: 30,
                cron_hour_utc: 2,
            },
            git: GitSettings {
                allowed_roots: vec!["/Users/fernandojulian/project/fjulian/fjulian.me".to_owned()],
                allowed_ssh_hosts: vec!["github.com".to_owned()],
                allowed_ssh_repositories: vec!["nandocoeg2/fjulian.me.git".to_owned()],
                excluded_directories: vec![
                    ".git".to_owned(),
                    ".astro".to_owned(),
                    ".next".to_owned(),
                    "node_modules".to_owned(),
                    "dist".to_owned(),
                    "build".to_owned(),
                    "coverage".to_owned(),
                    "target".to_owned(),
                ],
                max_file_bytes: 1_000_000,
                chunk_max_lines: 120,
                chunk_overlap_lines: 20,
                trufflehog_executable: "trufflehog".to_owned(),
                trufflehog_include_paths_file: "config/trufflehog-include-paths.txt".to_owned(),
                trufflehog_exclude_paths_file: "config/trufflehog-exclude-paths.txt".to_owned(),
                trufflehog_timeout_seconds: 300,
            },
            worker: WorkerSettings {
                poll_interval_ms: 2_000,
                lease_seconds: 900,
                identity: "queria-git-ingestion".to_owned(),
            },
        }
    }

    pub fn validate(&self) -> QueriaResult<()> {
        parse_addr("QUERIA_API_ADDR", &self.api_addr)?;
        parse_addr("QUERIA_MCP_ADDR", &self.mcp_addr)?;
        parse_addr("QUERIA_WORKER_HEALTH_ADDR", &self.worker_health_addr)?;
        parse_url("QUERIA_PUBLIC_BASE_URL", &self.public_base_url)?;
        parse_url("QUERIA_QDRANT_URL", &self.qdrant.url)?;
        parse_url("QUERIA_MINIO_ENDPOINT", &self.minio.endpoint)?;

        if self.environment != "local"
            && (self.embedding.voyage_api_key.trim().is_empty()
                || self.qdrant.api_key.trim().is_empty())
        {
            return Err(QueriaError::Config(
                "VOYAGE_API_KEY and QDRANT_API_KEY are required outside local".to_owned(),
            ));
        }
        if self.embedding.model.trim().is_empty()
            || self.embedding.profile_version.trim().is_empty()
            || self.qdrant.collection.trim().is_empty()
            || self.qdrant.vector_name.trim().is_empty()
        {
            return Err(QueriaError::Config(
                "embedding and Qdrant identifiers must not be blank".to_owned(),
            ));
        }
        if !matches!(self.embedding.dimension, 256 | 512 | 1024 | 2048) {
            return Err(QueriaError::Config(
                "QUERIA_EMBEDDING_DIMENSION must be 256, 512, 1024, or 2048".to_owned(),
            ));
        }
        if self.embedding.batch_size == 0
            || self.embedding.batch_size > 128
            || self.embedding.timeout_seconds == 0
            || self.embedding.max_retries > 10
            || self.embedding.request_interval_ms > 3_600_000
            || self.embedding.retry_backoff_base_seconds == 0
            || self.embedding.retry_backoff_max_seconds < self.embedding.retry_backoff_base_seconds
            || self.embedding.retry_backoff_max_seconds > 3_600
            || self.retrieval.rrf_k == 0
            || self.retrieval.candidate_multiplier == 0
            || self.retrieval.candidate_cap < 20
        {
            return Err(QueriaError::Config(
                "embedding and retrieval numeric limits are invalid".to_owned(),
            ));
        }

        if !self.database_url.starts_with("postgres://")
            && !self.database_url.starts_with("postgresql://")
        {
            return Err(QueriaError::Config(
                "QUERIA_DATABASE_URL must be a postgres URL".to_owned(),
            ));
        }

        if self.setup_token.len() < 24 || self.setup_token.starts_with("change-me") {
            return Err(QueriaError::Config(
                "QUERIA_SETUP_TOKEN must be replaced with a strong one-time token".to_owned(),
            ));
        }

        if !self.first_admin_email.contains('@') {
            return Err(QueriaError::Config(
                "QUERIA_FIRST_ADMIN_EMAIL must be a valid email-like value".to_owned(),
            ));
        }

        validate_slug("QUERIA_FIRST_ORG_SLUG", &self.first_org_slug)?;
        if self.git.allowed_roots.is_empty()
            || self.git.allowed_ssh_hosts.is_empty()
            || self.git.allowed_ssh_repositories.is_empty()
        {
            return Err(QueriaError::Config(
                "Git ingestion allowlists must not be empty".to_owned(),
            ));
        }
        if self.max_body_bytes == 0 || self.max_body_bytes > 1_000_000 {
            return Err(QueriaError::Config(
                "QUERIA_MAX_BODY_BYTES must be between 1 and 1000000".to_owned(),
            ));
        }
        if self.git.max_file_bytes == 0
            || self.git.chunk_max_lines == 0
            || self.git.chunk_overlap_lines >= self.git.chunk_max_lines
            || self.worker.poll_interval_ms == 0
            || self.worker.lease_seconds == 0
        {
            return Err(QueriaError::Config(
                "Git ingestion numeric limits are invalid".to_owned(),
            ));
        }
        if self.worker.identity.trim().is_empty()
            || self.git.trufflehog_executable.trim().is_empty()
            || self.git.trufflehog_include_paths_file.trim().is_empty()
            || self.git.trufflehog_exclude_paths_file.trim().is_empty()
            || self.git.trufflehog_timeout_seconds == 0
        {
            return Err(QueriaError::Config(
                "worker identity and TruffleHog configuration are required".to_owned(),
            ));
        }
        Ok(())
    }
}

fn read_env(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn read_csv_env(key: &str, default: &[String]) -> Vec<String> {
    env::var(key)
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn read_number_env<T>(key: &str, default: T) -> QueriaResult<T>
where
    T: std::str::FromStr,
{
    let Some(value) = env::var(key).ok().filter(|value| !value.trim().is_empty()) else {
        return Ok(default);
    };
    value
        .parse::<T>()
        .map_err(|_| QueriaError::Config(format!("{key} must be a positive integer")))
}

fn parse_addr(key: &str, value: &str) -> QueriaResult<SocketAddr> {
    value
        .parse::<SocketAddr>()
        .map_err(|_| QueriaError::Config(format!("{key} must be host:port")))
}

fn parse_url(key: &str, value: &str) -> QueriaResult<Url> {
    Url::parse(value).map_err(|_| QueriaError::Config(format!("{key} must be a valid URL")))
}

fn validate_slug(key: &str, value: &str) -> QueriaResult<()> {
    let valid_len = (3..=64).contains(&value.len());
    let valid_chars = value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    let valid_edges = value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());

    if valid_len && valid_chars && valid_edges {
        Ok(())
    } else {
        Err(QueriaError::Config(format!(
            "{key} must be a lowercase slug with digits or hyphens"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_defaults_use_reserved_ports() {
        let config = AppConfig::default_local();

        assert_eq!(config.api_addr, "127.0.0.1:17671");
        assert_eq!(config.mcp_addr, "127.0.0.1:17672");
        assert_eq!(config.worker_health_addr, "127.0.0.1:17673");
        assert!(config.database_url.contains("17675"));
        assert!(config.qdrant.url.ends_with(":17676"));
        assert_eq!(config.git.chunk_max_lines, 120);
        assert_eq!(config.git.chunk_overlap_lines, 20);
        assert_eq!(config.git.max_file_bytes, 1_000_000);
        assert_eq!(config.embedding.model, "voyage-4");
        assert_eq!(config.embedding.dimension, 1024);
        assert_eq!(config.embedding.profile_version, "voyage-4-1024-v1");
        assert_eq!(config.embedding.batch_size, 64);
        assert_eq!(config.embedding.request_interval_ms, 0);
        assert_eq!(config.embedding.retry_backoff_base_seconds, 30);
        assert_eq!(config.embedding.retry_backoff_max_seconds, 600);
        assert_eq!(config.qdrant.collection, "queria_local_chunks_v1");
        assert_eq!(config.qdrant.vector_name, "dense_v1");
        assert_eq!(config.retrieval.rrf_k, 60);
        assert_eq!(config.retrieval.candidate_multiplier, 4);
        assert_eq!(config.retrieval.candidate_cap, 100);
        assert_eq!(config.max_body_bytes, 20_000);
        assert!(
            config
                .git
                .allowed_ssh_repositories
                .contains(&"nandocoeg2/fjulian.me.git".to_owned())
        );
    }

    /// IMP-23: zero or absurd max_body_bytes rejected at config validate.
    #[test]
    fn validation_rejects_invalid_max_body_bytes() {
        let mut config = AppConfig::default_local();
        config.setup_token = "a-strong-setup-token-for-tests!!".to_owned();
        config.max_body_bytes = 0;
        assert!(matches!(
            config.validate().expect_err("zero max_body_bytes"),
            QueriaError::Config(_)
        ));

        let mut config = AppConfig::default_local();
        config.setup_token = "a-strong-setup-token-for-tests!!".to_owned();
        config.max_body_bytes = 1_000_001;
        assert!(matches!(
            config.validate().expect_err("too-large max_body_bytes"),
            QueriaError::Config(_)
        ));
    }

    #[test]
    fn local_default_service_ports_are_valid_socket_addresses() {
        let config = AppConfig::default_local();

        for addr in [
            &config.api_addr,
            &config.mcp_addr,
            &config.worker_health_addr,
        ] {
            addr.parse::<std::net::SocketAddr>()
                .expect("default service address must bind locally");
        }
    }

    #[test]
    fn validation_rejects_placeholder_setup_token() {
        let config = AppConfig::default_local();

        let err = config
            .validate()
            .expect_err("placeholder token must be rejected");

        assert!(matches!(err, QueriaError::Config(_)));
    }

    #[test]
    fn validation_rejects_unsupported_embedding_dimension() {
        let mut config = AppConfig::default_local();
        config.setup_token = "********************************".to_owned();
        config.embedding.dimension = 768;

        let err = config
            .validate()
            .expect_err("unsupported embedding dimension must be rejected");

        assert!(matches!(err, QueriaError::Config(_)));
    }

    #[test]
    fn validation_requires_provider_keys_outside_local() {
        let mut config = AppConfig::default_local();
        config.environment = "dev".to_owned();
        config.setup_token = "********************************".to_owned();

        let err = config
            .validate()
            .expect_err("remote environments require provider keys");

        assert!(matches!(err, QueriaError::Config(_)));
    }

    #[test]
    fn validation_rejects_invalid_embedding_retry_backoff() {
        let mut config = AppConfig::default_local();
        config.setup_token = "********************************".to_owned();
        config.embedding.retry_backoff_base_seconds = 0;

        let err = config
            .validate()
            .expect_err("zero retry base must be rejected");

        assert!(matches!(err, QueriaError::Config(_)));

        let mut config = AppConfig::default_local();
        config.setup_token = "********************************".to_owned();
        config.embedding.retry_backoff_base_seconds = 60;
        config.embedding.retry_backoff_max_seconds = 30;

        let err = config
            .validate()
            .expect_err("max retry backoff below base must be rejected");

        assert!(matches!(err, QueriaError::Config(_)));
    }

    #[test]
    fn validation_rejects_excessive_embedding_request_interval() {
        let mut config = AppConfig::default_local();
        config.setup_token = "********************************".to_owned();
        config.embedding.request_interval_ms = 3_600_001;

        let err = config
            .validate()
            .expect_err("excessive embedding request interval must be rejected");

        assert!(matches!(err, QueriaError::Config(_)));
    }

    #[test]
    fn env_reader_uses_default_for_blank_values() {
        unsafe {
            env::set_var("QUERIA_TEST_BLANK_VALUE", " ");
        }

        let value = read_env("QUERIA_TEST_BLANK_VALUE", "fallback");

        unsafe {
            env::remove_var("QUERIA_TEST_BLANK_VALUE");
        }

        assert_eq!(value, "fallback");
    }

    #[test]
    fn trufflehog_config_files_exist_in_repo() {
        let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let include = workspace_root.join("config/trufflehog-include-paths.txt");
        let exclude = workspace_root.join("config/trufflehog-exclude-paths.txt");
        assert!(
            include.is_file(),
            "missing {} (needed for image COPY)",
            include.display()
        );
        assert!(
            exclude.is_file(),
            "missing {} (needed for image COPY)",
            exclude.display()
        );

        let defaults = AppConfig::default_local();
        assert_eq!(
            defaults.git.trufflehog_include_paths_file,
            "config/trufflehog-include-paths.txt"
        );
        assert_eq!(
            defaults.git.trufflehog_exclude_paths_file,
            "config/trufflehog-exclude-paths.txt"
        );
    }

    #[test]
    fn dockerfile_bakes_trufflehog_config_into_image() {
        let dockerfile_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Dockerfile");
        let dockerfile = std::fs::read_to_string(&dockerfile_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", dockerfile_path.display()));
        assert!(
            dockerfile.contains(
                "COPY config/trufflehog-include-paths.txt /config/trufflehog-include-paths.txt"
            ),
            "Dockerfile must COPY include paths into /config"
        );
        assert!(
            dockerfile.contains(
                "COPY config/trufflehog-exclude-paths.txt /config/trufflehog-exclude-paths.txt"
            ),
            "Dockerfile must COPY exclude paths into /config"
        );
        assert!(
            dockerfile.contains(
                "QUERIA_TRUFFLEHOG_INCLUDE_PATHS_FILE=/config/trufflehog-include-paths.txt"
            ),
            "Dockerfile must set absolute include path env"
        );
        assert!(
            dockerfile.contains(
                "QUERIA_TRUFFLEHOG_EXCLUDE_PATHS_FILE=/config/trufflehog-exclude-paths.txt"
            ),
            "Dockerfile must set absolute exclude path env"
        );
    }
}
