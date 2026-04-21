use crate::cli::{ConfigAction, ConfigCommand};
use anyhow::Result;
use sivtr_core::config::SivtrConfig;
use sivtr_core::export::editor;

/// Execute config subcommands.
pub fn execute(cmd: ConfigCommand) -> Result<()> {
    match cmd.action {
        Some(ConfigAction::Show) | None => {
            let path = SivtrConfig::config_path()?;
            println!("Config file: {}", path.display());
            println!();

            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                println!("{content}");
            } else {
                println!("(file does not exist 鈥?using defaults)");
                println!();
                let config = SivtrConfig::default();
                let content = sivtr_core::config::to_toml_string(&config)?;
                println!("{content}");
                println!();
                println!("Run `sivtr config init` to create the config file.");
            }
        }
        Some(ConfigAction::Init) => {
            let path = SivtrConfig::init_default()?;
            println!("Config file created: {}", path.display());
        }
        Some(ConfigAction::Edit) => {
            let path = SivtrConfig::init_default()?;
            let ed = editor::resolve_editor()?;
            println!("Opening {} in {}...", path.display(), ed);

            let parts: Vec<&str> = ed.split_whitespace().collect();
            let (program, extra_args) = parts.split_first().unwrap();
            std::process::Command::new(program)
                .args(extra_args)
                .arg(&path)
                .status()?;
        }
    }
    Ok(())
}
