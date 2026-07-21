//! Resolve edge/token/slug from flags, env, and user config profiles.

use crate::config::{self, UserConfig};
use anyhow::{Context, Result, bail};

pub const DEFAULT_EDGE_URL: &str = "http://127.0.0.1:17674";

#[derive(Debug, Clone)]
pub struct ResolvedCredentials {
    #[allow(dead_code)]
    pub profile: Option<String>,
    pub edge_url: String,
    pub mcp_url: String,
    pub agent_token: Option<String>,
    #[allow(dead_code)]
    pub project_slug: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResolveOpts {
    /// Global CLI `--profile`.
    pub profile: Option<String>,
    /// When set, token is read from this **env var name** (index-here --token-env).
    pub token_env: Option<String>,
    /// When set, edge is read from this **env var name** (index-here --edge-url-env).
    pub edge_url_env: Option<String>,
    /// If true, missing token is an error.
    pub require_token: bool,
}

pub fn resolve(opts: ResolveOpts) -> Result<ResolvedCredentials> {
    let path = config::config_path()?;
    let cfg = UserConfig::load_or_default(&path)?;

    let profile_name = opts
        .profile
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
        .or_else(|| {
            std::env::var("QUERIA_PROFILE")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| cfg.active_profile.clone());

    let file_profile = profile_name.as_ref().and_then(|n| cfg.profile(n).cloned());

    // Token: named env (if any), then QUERIA_AGENT_TOKEN, then profile file.
    let agent_token = opts
        .token_env
        .as_ref()
        .and_then(|env_name| {
            std::env::var(env_name)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("QUERIA_AGENT_TOKEN")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            file_profile
                .as_ref()
                .and_then(|p| p.agent_token.clone())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        });

    // Edge: named env (if any), then QUERIA_EDGE_URL, then profile file, then default.
    let edge_url = opts
        .edge_url_env
        .as_ref()
        .and_then(|env_name| {
            std::env::var(env_name)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("QUERIA_EDGE_URL")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            file_profile
                .as_ref()
                .and_then(|p| p.edge_url.clone())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_EDGE_URL.to_owned());

    let mcp_url = std::env::var("QUERIA_MCP_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            file_profile
                .as_ref()
                .and_then(|p| p.mcp_url.clone())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("{}/mcp", edge_url.trim_end_matches('/')));

    let project_slug = std::env::var("QUERIA_PROJECT_SLUG")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            file_profile
                .as_ref()
                .and_then(|p| p.project_slug.clone())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        });

    if opts.require_token && agent_token.is_none() {
        bail!(
            "missing agent token: run `queria-cli config` (set token) or export QUERIA_AGENT_TOKEN \
             (config path: {})",
            path.display()
        );
    }

    let _ = profile_name
        .as_ref()
        .map(|n| config::validate_profile_name(n))
        .transpose()
        .context("profile name")?;

    Ok(ResolvedCredentials {
        profile: profile_name,
        edge_url,
        mcp_url,
        agent_token,
        project_slug,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Profile, UserConfig};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn env_token_overrides_file() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("queria-cred-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut cfg = UserConfig {
            active_profile: Some("work".into()),
            ..Default::default()
        };
        cfg.profiles.insert(
            "work".into(),
            Profile {
                edge_url: Some("https://file.example".into()),
                agent_token: Some("qria_file".into()),
                ..Default::default()
            },
        );
        cfg.save(&path).unwrap();
        // SAFETY: test-only env mutation under ENV_LOCK.
        unsafe {
            std::env::set_var("QUERIA_CONFIG", &path);
            std::env::set_var("QUERIA_AGENT_TOKEN", "qria_env");
            std::env::remove_var("QUERIA_EDGE_URL");
        }
        let r = resolve(ResolveOpts {
            require_token: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(r.agent_token.as_deref(), Some("qria_env"));
        assert_eq!(r.edge_url, "https://file.example");
        unsafe {
            std::env::remove_var("QUERIA_AGENT_TOKEN");
            std::env::remove_var("QUERIA_CONFIG");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_token_errors_when_required() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("queria-cred2-{}", uuid::Uuid::now_v7()));
        let path = dir.join("empty.toml");
        std::fs::create_dir_all(&dir).unwrap();
        UserConfig::default().save(&path).unwrap();
        unsafe {
            std::env::set_var("QUERIA_CONFIG", &path);
            std::env::remove_var("QUERIA_AGENT_TOKEN");
        }
        let err = resolve(ResolveOpts {
            require_token: true,
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.to_string().contains("missing agent token"));
        unsafe {
            std::env::remove_var("QUERIA_CONFIG");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
