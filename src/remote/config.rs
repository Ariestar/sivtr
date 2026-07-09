//! `remotes.toml` — registry of remote sivtr devices.
//!
//! Maps a ref alias (the `desk` in `desk://terminal/...`) to a host/port/token,
//! so a remote WorkRef resolves to a concrete `sivtr serve` endpoint. Stored at
//! `<data_dir>/sivtr/remotes.toml` alongside the other config. Unregistered
//! aliases are an error (see WorkRef parsing) — there is no `host:port://`
//! shorthand, because the bearer token must live somewhere.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use sivtr_core::workspace;

/// A configured remote device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub token: String,
    /// Optional hint for the workspace cwd display on the client side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

fn default_port() -> u16 {
    7421
}

impl Remote {
    /// `http://host:port` — serve is plain HTTP (localhost default; TLS is a
    /// future concern, so the scheme is fixed for now).
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

/// All configured remotes, keyed by alias.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Remotes {
    #[serde(default)]
    pub remotes: BTreeMap<String, Remote>,
}

impl Remotes {
    pub fn path() -> Result<PathBuf> {
        Ok(workspace::data_dir().join("remotes.toml"))
    }

    /// Load remotes from disk; an absent file is an empty set (not an error).
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let remotes: Self =
            toml::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(remotes)
    }

    /// Save remotes to disk, creating the parent directory if needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("Failed to serialize remotes.toml")?;
        fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Look up a remote by alias.
    pub fn get(&self, alias: &str) -> Option<&Remote> {
        self.remotes.get(alias)
    }
}

/// Look up a remote alias in the on-disk registry.
pub fn lookup(alias: &str) -> Result<Remote> {
    let remotes = Remotes::load()?;
    remotes
        .get(alias)
        .cloned()
        .with_context(|| format!("unknown remote `{alias}`; register it with `sivtr remote add`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remotes_toml() {
        let text = r#"
[remotes.desk]
host = "desk.local"
port = 7421
token = "s-abc"

[remotes.laptop]
host = "192.168.1.20"
token = "s-xyz"
"#;
        let remotes: Remotes = toml::from_str(text).unwrap();
        assert_eq!(remotes.get("desk").unwrap().host, "desk.local");
        assert_eq!(remotes.get("laptop").unwrap().port, 7421); // default
        assert_eq!(
            remotes.get("desk").unwrap().base_url(),
            "http://desk.local:7421"
        );
    }

    #[test]
    fn empty_file_is_empty_remotes() {
        let remotes: Remotes = toml::from_str("").unwrap();
        assert!(remotes.remotes.is_empty());
    }
}
