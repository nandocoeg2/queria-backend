//! User config file (~/.config/queria/config.toml). Edit only via TUI (`queria-cli config`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

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
        let raw =
            fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse config {}", path.display()))
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
    let b = name.as_bytes();
    if b.is_empty() || b.len() > 64 || !b[0].is_ascii_alphanumeric() {
        bail!("invalid profile name {name:?}: 1–64 chars, start alphanumeric, then [A-Za-z0-9_-]");
    }
    if !b
        .iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
    {
        bail!("invalid profile name {name:?}");
    }
    Ok(())
}

/// `QUERIA_CONFIG` → `$XDG_CONFIG_HOME/queria/config.toml` → `~/.config/queria/config.toml`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn profile_name_validation() {
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("-x").is_err());
    }

    #[test]
    fn redact_hides_tail() {
        assert_eq!(redact_token("qria_abcdefghij"), "qria_abc…****");
    }

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("queria-cfg-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut cfg = UserConfig {
            active_profile: Some("work".into()),
            ..Default::default()
        };
        cfg.profile_mut("work").agent_token = Some("qria_secret".into());
        cfg.save(&path).unwrap();
        let loaded = UserConfig::load_or_default(&path).unwrap();
        assert_eq!(
            loaded.profile("work").unwrap().agent_token.as_deref(),
            Some("qria_secret")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_path_respects_queria_config_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("q-{}", uuid::Uuid::now_v7()));
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
