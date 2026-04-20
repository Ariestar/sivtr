pub mod keys;

use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SivtrConfig {
    /// General settings.
    pub general: GeneralConfig,
    /// Editor settings.
    pub editor: EditorConfig,
    /// History settings.
    pub history: HistoryConfig,
}

/// General behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// How to open captured output: "tui" or "editor".
    /// - "tui": open in built-in TUI browser (default)
    /// - "editor": open directly in external editor
    pub open_mode: OpenMode,
    /// Preserve original ANSI colors in TUI display.
    pub preserve_colors: bool,
}

/// How sivtr opens captured output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenMode {
    /// Built-in TUI browser (default).
    Tui,
    /// Open directly in external editor.
    Editor,
}

/// Editor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// Editor command. If empty, auto-detect from PATH.
    /// Examples: "hx", "nvim", "vim", "code --wait"
    pub command: String,
}

/// History storage settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Whether to automatically save captured output to history.
    pub auto_save: bool,
    /// Maximum number of history entries to keep (0 = unlimited).
    pub max_entries: usize,
}

// --- Defaults ---

impl Default for SivtrConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            editor: EditorConfig::default(),
            history: HistoryConfig::default(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            open_mode: OpenMode::Tui,
            preserve_colors: true,
        }
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            command: String::new(), // empty = auto-detect
        }
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            auto_save: true,
            max_entries: 0, // unlimited
        }
    }
}

impl Default for OpenMode {
    fn default() -> Self {
        OpenMode::Tui
    }
}

// --- Loading / Saving ---

impl SivtrConfig {
    /// Load config from the default config file.
    /// If the file doesn't exist, return defaults.
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: SivtrConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    /// Save config to the default config file.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;
        Ok(())
    }

    /// Generate the default config file if it doesn't exist.
    /// Returns the path to the config file.
    pub fn init_default() -> Result<PathBuf> {
        let path = Self::config_path()?;
        if !path.exists() {
            let config = Self::default();
            config.save()?;
        }
        Ok(path)
    }

    /// Get the config file path.
    /// Windows: %APPDATA%/sivtr/config.toml
    /// macOS:   ~/Library/Application Support/sivtr/config.toml
    /// Linux:   ~/.config/sivtr/config.toml
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
        let current = config_dir.join("sivtr").join("config.toml");
        if current.exists() {
            return Ok(current);
        }

        let legacy = config_dir.join("sift").join("config.toml");
        if legacy.exists() {
            return Ok(legacy);
        }

        Ok(current)
    }
}

/// Serialize a SivtrConfig to a pretty TOML string.
pub fn to_toml_string(config: &SivtrConfig) -> Result<String> {
    toml::to_string_pretty(config).context("Failed to serialize config to TOML")
}
