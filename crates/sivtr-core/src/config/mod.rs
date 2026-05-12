pub mod keys;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SivtrConfig {
    /// General settings.
    pub general: GeneralConfig,
    /// Editor settings.
    pub editor: EditorConfig,
    /// History settings.
    pub history: HistoryConfig,
    /// Copy command settings.
    pub copy: CopyConfig,
    /// Codex session settings.
    pub codex: CodexConfig,
    /// Global hotkey settings.
    pub hotkey: HotkeyConfig,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenMode {
    /// Built-in TUI browser (default).
    #[default]
    Tui,
    /// Open directly in external editor.
    Editor,
}

/// Editor configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Copy command configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CopyConfig {
    /// Prompt profiles or literal prefixes used when detecting command lines.
    pub prompts: Vec<String>,

    #[serde(rename = "prompt_presets", skip_serializing)]
    pub legacy_prompt_presets: Vec<String>,
}

impl CopyConfig {
    pub fn prompt_values(&self) -> impl Iterator<Item = &String> {
        self.legacy_prompt_presets.iter().chain(self.prompts.iter())
    }
}

/// Codex session configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexConfig {
    /// Additional directories that contain exported Codex session JSONL trees.
    pub session_dirs: Vec<PathBuf>,
}

/// Global hotkey configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Hotkey chord used by `sivtr hotkey start`.
    pub chord: String,
}

// --- Defaults ---

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            open_mode: OpenMode::Tui,
            preserve_colors: true,
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

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            chord: "alt+y".to_string(),
        }
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
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_copy_prompt_config() {
        let config = SivtrConfig {
            copy: CopyConfig {
                prompts: vec!["arrow".to_string(), "mysh>".to_string(), "dev>".to_string()],
                legacy_prompt_presets: vec!["cmd".to_string()],
            },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).unwrap();

        assert!(toml.contains("[copy]"));
        assert!(toml.contains("prompts = ["));
        assert!(toml.contains("\"arrow\""));
        assert!(toml.contains("\"mysh>\""));
        assert!(toml.contains("\"dev>\""));
        assert!(!toml.contains("prompt_presets"));
    }

    #[test]
    fn serializes_hotkey_config() {
        let config = SivtrConfig {
            hotkey: HotkeyConfig {
                chord: "alt+y".to_string(),
            },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).unwrap();

        assert!(toml.contains("[hotkey]"));
        assert!(toml.contains("chord = \"alt+y\""));
    }

    #[test]
    fn serializes_codex_config() {
        let config = SivtrConfig {
            codex: CodexConfig {
                session_dirs: vec![PathBuf::from("/srv/sivtr/root-codex/sessions")],
            },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).unwrap();

        assert!(toml.contains("[codex]"));
        assert!(toml.contains("session_dirs = ["));
        assert!(toml.contains("/srv/sivtr/root-codex/sessions"));
    }
}
