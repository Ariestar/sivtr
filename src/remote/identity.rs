use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use iroh::SecretKey;
use sivtr_core::workspace;

#[derive(Debug, Clone)]
pub struct Identity {
    pub name: String,
    pub secret_key: SecretKey,
}

impl Identity {
    pub fn load_or_create() -> Result<Self> {
        let path = identity_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
            restrict_directory(parent)?;
        }
        let secret_key = if path.exists() {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid device identity at {}", path.display()))?;
            SecretKey::from_bytes(&bytes)
        } else {
            let secret_key = SecretKey::generate();
            write_secret(&path, &secret_key)?;
            secret_key
        };
        Ok(Self {
            name: device_name()?,
            secret_key,
        })
    }

    pub fn id(&self) -> String {
        self.secret_key.public().to_string()
    }
}

pub fn identity_path() -> PathBuf {
    workspace::data_dir().join("identity.key")
}

fn write_secret(path: &Path, secret_key: &SecretKey) -> Result<()> {
    let temporary = path.with_extension("key.tmp");
    std::fs::write(&temporary, secret_key.to_bytes())
        .with_context(|| format!("Failed to write {}", temporary.display()))?;
    restrict_file(&temporary)?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("Failed to install {}", path.display()))?;
    restrict_file(path)?;
    Ok(())
}

fn device_name() -> Result<String> {
    let name = std::env::var("SIVTR_DEVICE_NAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "sivtr-device".to_string());
    let normalized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        bail!("Device name is empty; set SIVTR_DEVICE_NAME");
    }
    Ok(normalized)
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<()> {
    Ok(())
}
