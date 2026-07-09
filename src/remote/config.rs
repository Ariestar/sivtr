//! `remotes.toml` — registry of remote sivtr devices.
//!
//! Maps a ref alias (the `desk` in `desk://terminal/...`) to either a TCP
//! `sivtr pair` endpoint (host/port/token) or an iroh ticket (zero-config,
//! cross-network). Stored at `<data_dir>/sivtr/remotes.toml`. Unregistered
//! aliases are an error — there is no `host:port://` shorthand, because the
//! bearer token / ticket must live somewhere.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use sivtr_core::workspace;

/// A configured remote device.
///
/// `Tcp` is the direct host:port transport (localhost/LAN); `Iroh` is the
/// zero-config encrypted transport (cross-NAT, relay-assisted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum Remote {
    Tcp {
        host: String,
        #[serde(default = "default_port")]
        port: u16,
        token: String,
    },
    Iroh {
        /// base64 endpoint-address ticket printed by `sivtr pair --iroh`.
        ticket: String,
    },
}

fn default_port() -> u16 {
    7421
}

impl Remote {
    /// A short label for `remote list`/`test`.
    pub fn kind(&self) -> &'static str {
        match self {
            Remote::Tcp { .. } => "tcp",
            Remote::Iroh { .. } => "iroh",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Remote::Tcp { host, port, .. } => format!("{host} (port {port})"),
            Remote::Iroh { ticket } => format!("iroh:{}…", &ticket[..ticket.len().min(8)]),
        }
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
    fn parses_tcp_and_iroh_remotes() {
        let text = r#"
[remotes.desk]
transport = "tcp"
host = "desk.local"
port = 7421
token = "s-abc"

[remotes.box]
transport = "iroh"
ticket = "eyJ0ZXN0IjoidGlja2V0In0="
"#;
        let remotes: Remotes = toml::from_str(text).unwrap();
        assert!(matches!(
            remotes.get("desk").unwrap(),
            Remote::Tcp { host, port, .. } if host == "desk.local" && *port == 7421
        ));
        assert_eq!(remotes.get("desk").unwrap().kind(), "tcp");
        assert_eq!(
            remotes.get("desk").unwrap().describe(),
            "desk.local (port 7421)"
        );
        assert!(matches!(remotes.get("box").unwrap(), Remote::Iroh { .. }));
        assert_eq!(remotes.get("box").unwrap().kind(), "iroh");
    }

    #[test]
    fn empty_file_is_empty_remotes() {
        let remotes: Remotes = toml::from_str("").unwrap();
        assert!(remotes.remotes.is_empty());
    }
}
