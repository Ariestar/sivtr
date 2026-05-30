use anyhow::Result;
use std::path::Path;

use crate::output;

pub fn execute() -> Result<()> {
    let mut checks = 0;
    let mut passed = 0;

    checks += 1;
    output::info("checking binary version");
    output::detail("version", format!("sivtr {}", env!("CARGO_PKG_VERSION")));
    passed += 1;

    checks += 1;
    output::info("checking config file");
    if let Some(config_dir) = dirs::config_dir() {
        let config_path = config_dir.join("sivtr").join("config.toml");
        if config_path.exists() {
            output::detail("found", config_path.display());
            passed += 1;
        } else {
            output::warning("config file is missing");
            output::hint("run `sivtr config init` to create it");
        }
    } else {
        output::warning("unable to determine config directory");
    }

    checks += 1;
    output::info("checking session log directory");
    if let Some(state_dir) =
        dirs::state_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
    {
        let session_dir = state_dir.join("sivtr");
        if session_dir.exists() {
            let count = std::fs::read_dir(&session_dir)
                .map(|d| d.count())
                .unwrap_or(0);
            output::detail(
                "found",
                format!("{} ({count} entries)", session_dir.display()),
            );
            passed += 1;
        } else {
            output::warning("session log directory is missing");
            output::hint("run `sivtr init <shell>`, restart the shell, then run a command");
        }
    } else {
        output::warning("unable to determine state or home directory");
    }

    checks += 1;
    output::info("checking shell hooks");
    if check_shell_hooks() {
        passed += 1;
    }

    checks += 1;
    output::info("checking provider sessions");
    check_providers();
    passed += 1;

    checks += 1;
    output::info("checking clipboard access");
    if check_clipboard() {
        output::success("clipboard is available");
        passed += 1;
    } else {
        output::warning("clipboard is unavailable");
        output::hint("copy commands may not work in this environment");
    }

    output::blank();
    if passed == checks {
        output::success(format!("all {checks} checks passed"));
    } else {
        output::warning(format!("{passed}/{checks} checks passed"));
    }

    Ok(())
}

fn check_shell_hooks() -> bool {
    let mut any_installed = false;

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            output::warning("cannot determine home directory");
            return false;
        }
    };

    for (name, rel_path, marker) in [
        ("bash", ".bashrc", "# >>> sivtr shell integration >>>"),
        ("zsh", ".zshrc", "# >>> sivtr shell integration >>>"),
    ] {
        let path = home.join(rel_path);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(marker) {
                    output::detail(name, "installed");
                    any_installed = true;
                } else {
                    output::detail(name, "not installed");
                }
            }
        } else {
            output::detail(name, "no profile file");
        }
    }

    if let Some(config_dir) = dirs::config_dir() {
        let nu_config = config_dir.join("nushell").join("config.nu");
        if nu_config.exists() {
            if let Ok(content) = std::fs::read_to_string(&nu_config) {
                if content.contains("# >>> sivtr shell integration >>>") {
                    output::detail("nushell", "installed");
                    any_installed = true;
                } else {
                    output::detail("nushell", "not installed");
                }
            }
        } else {
            output::detail("nushell", "no config file");
        }
    } else {
        output::detail("nushell", "unable to determine config directory");
    }

    for cmd in &["pwsh", "powershell"] {
        if let Ok(output) = std::process::Command::new(cmd)
            .args(["-NoProfile", "-Command", "Write-Output $PROFILE"])
            .output()
        {
            let profile = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !profile.is_empty() {
                let path = Path::new(&profile);
                if path.exists() {
                    if let Ok(content) = std::fs::read_to_string(path) {
                        let status = if content.contains("# >>> sivtr shell integration >>>") {
                            any_installed = true;
                            "installed"
                        } else {
                            "not installed"
                        };
                        output::detail(format!("powershell ({cmd})"), status);
                    }
                }
            }
        }
    }

    if !any_installed {
        output::hint("run `sivtr init bash` or `sivtr init zsh|powershell|nushell`");
    }

    any_installed
}

fn check_providers() {
    for spec in sivtr_core::ai::AgentProvider::all() {
        let provider = spec.provider.session_provider();
        match provider.list_recent_sessions(None) {
            Ok(s) if s.is_empty() => {
                output::detail(spec.provider.name(), "no sessions found");
            }
            Ok(s) => {
                output::detail(
                    spec.provider.name(),
                    format!("{} session(s) found", s.len()),
                );
            }
            Err(e) => {
                output::detail(spec.provider.name(), format!("error ({e})"));
            }
        }
    }
}

fn check_clipboard() -> bool {
    arboard::Clipboard::new().is_ok()
}
