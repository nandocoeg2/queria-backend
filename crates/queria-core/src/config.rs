use crate::{QueriaError, QueriaResult};
use serde::{Deserialize, Serialize};
use std::{env, net::SocketAddr};
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub environment: String,
    pub public_base_url: String,
    pub api_addr: String,
    pub mcp_addr: String,
    pub worker_health_addr: String,
    pub proxy_addr: String,
    pub database_url: String,
    pub qdrant_url: String,
    pub minio_endpoint: String,
    pub minio_bucket: String,
    pub setup_token: String,
    pub first_admin_email: String,
    pub first_org_slug: String,
    pub log_level: String,
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
            proxy_addr: read_env("QUERIA_PROXY_ADDR", &defaults.proxy_addr),
            database_url: read_env("QUERIA_DATABASE_URL", &defaults.database_url),
            qdrant_url: read_env("QUERIA_QDRANT_URL", &defaults.qdrant_url),
            minio_endpoint: read_env("QUERIA_MINIO_ENDPOINT", &defaults.minio_endpoint),
            minio_bucket: read_env("QUERIA_MINIO_BUCKET", &defaults.minio_bucket),
            setup_token: read_env("QUERIA_SETUP_TOKEN", &defaults.setup_token),
            first_admin_email: read_env("QUERIA_FIRST_ADMIN_EMAIL", &defaults.first_admin_email),
            first_org_slug: read_env("QUERIA_FIRST_ORG_SLUG", &defaults.first_org_slug),
            log_level: read_env("QUERIA_LOG_LEVEL", &defaults.log_level),
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
            proxy_addr: "127.0.0.1:17674".to_owned(),
            database_url: "postgres://queria:queria@127.0.0.1:17675/queria".to_owned(),
            qdrant_url: "http://127.0.0.1:17676".to_owned(),
            minio_endpoint: "http://127.0.0.1:17678".to_owned(),
            minio_bucket: "queria-local".to_owned(),
            setup_token: "change-me-one-time-setup-token".to_owned(),
            first_admin_email: "nando@fjulian.id".to_owned(),
            first_org_slug: "fjulian".to_owned(),
            log_level: "info".to_owned(),
        }
    }

    pub fn validate(&self) -> QueriaResult<()> {
        parse_addr("QUERIA_API_ADDR", &self.api_addr)?;
        parse_addr("QUERIA_MCP_ADDR", &self.mcp_addr)?;
        parse_addr("QUERIA_WORKER_HEALTH_ADDR", &self.worker_health_addr)?;
        parse_addr("QUERIA_PROXY_ADDR", &self.proxy_addr)?;
        parse_url("QUERIA_PUBLIC_BASE_URL", &self.public_base_url)?;
        parse_url("QUERIA_QDRANT_URL", &self.qdrant_url)?;
        parse_url("QUERIA_MINIO_ENDPOINT", &self.minio_endpoint)?;

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
        Ok(())
    }
}

fn read_env(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
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
        assert_eq!(config.proxy_addr, "127.0.0.1:17674");
        assert!(config.database_url.contains("17675"));
        assert!(config.qdrant_url.ends_with(":17676"));
    }

    #[test]
    fn local_default_service_ports_are_valid_socket_addresses() {
        let config = AppConfig::default_local();

        for addr in [
            &config.api_addr,
            &config.mcp_addr,
            &config.worker_health_addr,
            &config.proxy_addr,
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
}
