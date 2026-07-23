pub mod agent_helpers;
pub mod auth;
pub mod config;
pub mod contracts;
pub mod error;
pub mod evaluation;
pub mod ids;
pub mod model;
pub mod observability;

pub use agent_helpers::{
    AGENT_HTTP_RETRIEVE_LIMIT_DEFAULT, AGENT_HTTP_RETRIEVE_LIMIT_MAX, AGENT_RETRIEVE_LIMIT_DEFAULT,
    agent_include_global, agent_include_needs_review, agent_include_scratch,
    clamp_agent_http_retrieve_limit, is_valid_project_slug, parse_agent_bearer_token,
};
pub use config::{
    AppConfig, BackupSettings, EmbeddingSettings, GitSettings, MinioSettings, QdrantSettings,
    RetrievalSettings, WorkerSettings,
};
pub use contracts::{normalize_memory_body_for_hash, scratch_content_hash, validate_memory_body};
pub use error::{QueriaError, QueriaResult};
pub use observability::init_json_tracing;
