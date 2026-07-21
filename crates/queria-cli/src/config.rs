//! User config file for multi-profile agent credentials (queria-cli only).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

const PROFILE_NAME_RE: &str = r"^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub active_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
}

impl UserConfig {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: UserConfig = toml::from_str(&raw)
            .with_context(|| format!("parse config {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create config dir {}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }
        let raw = toml::to_string_pretty(self).context("serialize config")?;
        fs::write(path, raw.as_bytes())
            .with_context(|| format!("write config {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut Profile {
        self.profiles.entry(name.to_owned()).or_default()
    }
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    let re = regex_lite_profile_name(name);
    if !re {
        bail!(
            "invalid profile name {name:?}: use 1–64 chars, start alphanumeric, then [A-Za-z0-9_-]"
        );
    }
    Ok(())
}

fn regex_lite_profile_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() || b.len() > 64 {
        return false;
    }
    let first = b[0];
    if !(first.is_ascii_alphanumeric()) {
        return false;
    }
    b.iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
}

/// Resolve config file path. `QUERIA_CONFIG` wins when set.
pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("QUERIA_CONFIG") {
        let p = p.trim();
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let xdg = xdg.trim();
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("queria").join("config.toml"));
        }
    }
    let home = std::env::var("HOME").context("HOME not set; set QUERIA_CONFIG or HOME")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("queria")
        .join("config.toml"))
}

pub fn redact_token(token: &str) -> String {
    let t = token.trim();
    if t.len() <= 8 {
        return "****".to_owned();
    }
    format!("{}…****", &t[..t.len().min(8)])
}

pub fn is_tty() -> bool {
    io::stdout().is_terminal()
}

pub fn normalize_key(key: &str) -> Result<&'static str> {
    match key.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "edge-url" | "edge" => Ok("edge-url"),
        "token" | "agent-token" => Ok("token"),
        "mcp-url" | "mcp" => Ok("mcp-url"),
        "project-slug" | "slug" => Ok("project-slug"),
        other => bail!("unknown config key {other:?}; use edge-url|token|mcp-url|project-slug"),
    }
}

pub fn run_noninteractive(
    cmd: ConfigCommand,
    profile_override: Option<&str>,
) -> Result<()> {
    let path = config_path()?;
    match cmd {
        ConfigCommand::Path => {
            println!("{}", path.display());
            Ok(())
        }
        ConfigCommand::List => {
            let cfg = UserConfig::load_or_default(&path)?;
            if cfg.profiles.is_empty() {
                println!("(no profiles)");
                return Ok(());
            }
            for name in cfg.profiles.keys() {
                let mark = if cfg.active_profile.as_deref() == Some(name.as_str()) {
                    "*"
                } else {
                    " "
                };
                println!("{mark} {name}");
            }
            Ok(())
        }
        ConfigCommand::Show { name } => {
            let cfg = UserConfig::load_or_default(&path)?;
            let name = resolve_profile_name(&cfg, name.as_deref(), profile_override)?;
            let p = cfg
                .profile(&name)
                .with_context(|| format!("profile {name:?} not found"))?;
            println!("profile: {name}");
            println!("edge_url: {}", p.edge_url.as_deref().unwrap_or(""));
            println!(
                "mcp_url: {}",
                p.mcp_url
                    .clone()
                    .or_else(|| p.edge_url.as_ref().map(|e| format!(
                        "{}/mcp",
                        e.trim_end_matches('/')
                    )))
                    .unwrap_or_default()
            );
            println!("project_slug: {}", p.project_slug.as_deref().unwrap_or(""));
            println!(
                "agent_token: {}",
                p.agent_token
                    .as_deref()
                    .map(redact_token)
                    .unwrap_or_else(|| "(unset)".to_owned())
            );
            Ok(())
        }
        ConfigCommand::Use { name } => {
            validate_profile_name(&name)?;
            let mut cfg = UserConfig::load_or_default(&path)?;
            if !cfg.profiles.contains_key(&name) {
                bail!("profile {name:?} not found; config set token … --profile {name}");
            }
            cfg.active_profile = Some(name.clone());
            cfg.save(&path)?;
            println!("active_profile = {name}");
            Ok(())
        }
        ConfigCommand::Set {
            key,
            value,
            profile,
        } => {
            let key = normalize_key(&key)?;
            let mut cfg = UserConfig::load_or_default(&path)?;
            let name = profile
                .or_else(|| profile_override.map(|s| s.to_owned()))
                .or_else(|| cfg.active_profile.clone())
                .unwrap_or_else(|| "default".to_owned());
            validate_profile_name(&name)?;
            let p = cfg.profile_mut(&name);
            match key {
                "edge-url" => p.edge_url = Some(value.trim().to_owned()),
                "token" => p.agent_token = Some(value.trim().to_owned()),
                "mcp-url" => p.mcp_url = Some(value.trim().to_owned()),
                "project-slug" => p.project_slug = Some(value.trim().to_owned()),
                _ => unreachable!(),
            }
            if cfg.active_profile.is_none() {
                cfg.active_profile = Some(name.clone());
            }
            cfg.save(&path)?;
            if key == "token" {
                println!("set {key} on profile {name} ({})", redact_token(value.trim()));
            } else {
                println!("set {key} on profile {name}");
            }
            Ok(())
        }
        ConfigCommand::Unset { key, profile } => {
            let key = normalize_key(&key)?;
            let mut cfg = UserConfig::load_or_default(&path)?;
            let name = resolve_profile_name(&cfg, profile.as_deref(), profile_override)?;
            let p = cfg
                .profiles
                .get_mut(&name)
                .with_context(|| format!("profile {name:?} not found"))?;
            match key {
                "edge-url" => p.edge_url = None,
                "token" => p.agent_token = None,
                "mcp-url" => p.mcp_url = None,
                "project-slug" => p.project_slug = None,
                _ => unreachable!(),
            }
            cfg.save(&path)?;
            println!("unset {key} on profile {name}");
            Ok(())
        }
        ConfigCommand::Delete { name } => {
            let mut cfg = UserConfig::load_or_default(&path)?;
            if cfg.profiles.remove(&name).is_none() {
                bail!("profile {name:?} not found");
            }
            if cfg.active_profile.as_deref() == Some(name.as_str()) {
                cfg.active_profile = cfg.profiles.keys().next().cloned();
            }
            cfg.save(&path)?;
            println!("deleted profile {name}");
            Ok(())
        }
        ConfigCommand::Env { profile } => {
            let cfg = UserConfig::load_or_default(&path)?;
            let name = resolve_profile_name(&cfg, profile.as_deref(), profile_override)?;
            let p = cfg
                .profile(&name)
                .with_context(|| format!("profile {name:?} not found"))?;
            let edge = p
                .edge_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("http://127.0.0.1:17674");
            let mcp = p
                .mcp_url
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("{}/mcp", edge.trim_end_matches('/')));
            eprintln!("# warning: prints secrets to stdout (profile {name})");
            if let Some(t) = p.agent_token.as_deref().filter(|s| !s.is_empty()) {
                println!("export QUERIA_AGENT_TOKEN='{}'", t.replace('\'', r"'\''"));
            } else {
                eprintln!("# agent_token unset");
            }
            println!("export QUERIA_EDGE_URL='{}'", edge.replace('\'', r"'\''"));
            println!("export QUERIA_MCP_URL='{}'", mcp.replace('\'', r"'\''"));
            if let Some(slug) = p.project_slug.as_deref().filter(|s| !s.is_empty()) {
                println!(
                    "export QUERIA_PROJECT_SLUG='{}'",
                    slug.replace('\'', r"'\''")
                );
            }
            let _ = io::stderr().flush();
            Ok(())
        }
    }
}

pub fn resolve_profile_name(
    cfg: &UserConfig,
    explicit: Option<&str>,
    cli_profile: Option<&str>,
) -> Result<String> {
    if let Some(n) = explicit.filter(|s| !s.is_empty()) {
        validate_profile_name(n)?;
        return Ok(n.to_owned());
    }
    if let Some(n) = cli_profile.filter(|s| !s.is_empty()) {
        validate_profile_name(n)?;
        return Ok(n.to_owned());
    }
    if let Ok(n) = std::env::var("QUERIA_PROFILE") {
        let n = n.trim();
        if !n.is_empty() {
            validate_profile_name(n)?;
            return Ok(n.to_owned());
        }
    }
    if let Some(n) = cfg.active_profile.as_deref().filter(|s| !s.is_empty()) {
        return Ok(n.to_owned());
    }
    if let Some(n) = cfg.profiles.keys().next() {
        return Ok(n.clone());
    }
    bail!("no profile; run `queria-cli config` or `config set token …`")
}

#[derive(Debug, Clone)]
pub enum ConfigCommand {
    Path,
    List,
    Show { name: Option<String> },
    Use { name: String },
    Set {
        key: String,
        value: String,
        profile: Option<String>,
    },
    Unset {
        key: String,
        profile: Option<String>,
    },
    Delete { name: String },
    Env { profile: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn profile_name_validation() {
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("a").is_ok());
        assert!(validate_profile_name("Work_1-2").is_ok());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("-x").is_err());
        assert!(validate_profile_name("has space").is_err());
        let _ = PROFILE_NAME_RE;
    }

    #[test]
    fn redact_hides_tail() {
        assert_eq!(redact_token("qria_abcdefghij"), "qria_abc…****");
        assert_eq!(redact_token("short"), "****");
    }

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("queria-cfg-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut cfg = UserConfig::default();
        cfg.active_profile = Some("work".into());
        cfg.profile_mut("work").edge_url = Some("https://example.com".into());
        cfg.profile_mut("work").agent_token = Some("qria_secret".into());
        cfg.save(&path).unwrap();
        let loaded = UserConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.active_profile.as_deref(), Some("work"));
        assert_eq!(
            loaded.profile("work").unwrap().agent_token.as_deref(),
            Some("qria_secret")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_path_respects_queria_config_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("queria-cfg-path-{}", uuid::Uuid::now_v7()));
        let path = dir.join("custom.toml");
        // SAFETY: test-only env mutation under ENV_LOCK.
        unsafe {
            std::env::set_var("QUERIA_CONFIG", &path);
        }
        let got = config_path().unwrap();
        unsafe {
            std::env::remove_var("QUERIA_CONFIG");
        }
        assert_eq!(got, path);
    }
}
