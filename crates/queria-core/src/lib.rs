pub mod auth;
pub mod config;
pub mod contracts;
pub mod error;
pub mod evaluation;
pub mod ids;
pub mod model;
pub mod observability;

pub use config::{
    AppConfig, BackupSettings, EmbeddingSettings, GitSettings, MinioSettings, QdrantSettings,
    RetrievalSettings, WorkerSettings,
};
pub use contracts::{normalize_memory_body_for_hash, scratch_content_hash, validate_memory_body};
pub use error::{QueriaError, QueriaResult};
pub use observability::init_json_tracing;
