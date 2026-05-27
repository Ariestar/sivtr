use anyhow::Result;
use std::path::Path;

pub fn execute() -> Result<()> {
    let mut checks = 0;
    let mut passed = 0;

    // Binary version
    checks += 1;
    eprintln!("[check] binary version");
    eprintln!("  sivtr {}", env!("CARGO_PKG_VERSION"));
    passed += 1;

    // Config file
    checks += 1;
    eprintln!("[check] config file");
    if let Some(config_dir) = dirs::config_dir() {
        let config_path = config_dir.join("sivtr").join("config.toml");
        if config_path.exists() {
            eprintln!("  found: {}", config_path.display());
            passed += 1;
        } else {
            eprintln!("  missing (run `sivtr config init` to create)");
        }
    } else {
        eprintln!("  unable to determine config directory");
    }

    // Session log directory
    checks += 1;
    eprintln!("[check] session log directory");
    if let Some(state_dir) =
        dirs::state_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
    {
        let session_dir = state_dir.join("sivtr");
        if session_dir.exists() {
            let count = std::fs::read_dir(&session_dir)
                .map(|d| d.count())
                .unwrap_or(0);
            eprintln!(
                "  found: {} ({} session logs)",
                session_dir.display(),
                count
            );
            passed += 1;
        } else {
            eprintln!("  missing (will be created on first `sivtr init` + terminal restart)");
        }
    } else {
        eprintln!("  unable to determine state or home directory");
    }

    // Shell hooks
    checks += 1;
    eprintln!("[check] shell hooks");
    let hooks_ok = check_shell_hooks();
    if hooks_ok {
        passed += 1;
    }

    // Provider data
    checks += 1;
    eprintln!("[check] provider sessions");
    check_providers();
    passed += 1;

    // Clipboard
    checks += 1;
    eprintln!("[check] clipboard access");
    if check_clipboard() {
        eprintln!("  ok");
        passed += 1;
    } else {
        eprintln!("  unavailable (copy commands may not work)");
    }

    eprintln!();
    if passed == checks {
        eprintln!("sivtr: all {checks} checks passed");
    } else {
        eprintln!("sivtr: {passed}/{checks} checks passed");
    }

    Ok(())
}

fn check_shell_hooks() -> bool {
    let mut any_installed = false;

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("  cannot determine home directory");
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
                    eprintln!("  {name}: installed");
                    any_installed = true;
                } else {
                    eprintln!("  {name}: not installed");
                }
            }
        } else {
            eprintln!("  {name}: no profile file");
        }
    }

    // Nushell
    if let Some(config_dir) = dirs::config_dir() {
        let nu_config = config_dir.join("nushell").join("config.nu");
        if nu_config.exists() {
            if let Ok(content) = std::fs::read_to_string(&nu_config) {
                if content.contains("# >>> sivtr shell integration >>>") {
                    eprintln!("  nushell: installed");
                    any_installed = true;
                } else {
                    eprintln!("  nushell: not installed");
                }
            }
        } else {
            eprintln!("  nushell: no config file");
        }
    } else {
        eprintln!("  nushell: unable to determine config directory");
    }

    // PowerShell
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
                        if content.contains("# >>> sivtr shell integration >>>") {
                            eprintln!("  powershell ({cmd}): installed");
                            any_installed = true;
                        } else {
                            eprintln!("  powershell ({cmd}): not installed");
                        }
                    }
                }
            }
        }
    }

    if !any_installed {
        eprintln!("  hint: run `sivtr init bash` (or zsh/pwsh/nushell) to install hooks");
    }

    any_installed
}

fn check_providers() {
    for spec in sivtr_core::ai::AgentProvider::all() {
        let provider = spec.provider.session_provider();
        match provider.list_recent_sessions(None) {
            Ok(s) if s.is_empty() => {
                eprintln!("  {}: no sessions found", spec.provider.name());
            }
            Ok(s) => {
                eprintln!("  {}: {} session(s) found", spec.provider.name(), s.len());
            }
            Err(e) => {
                eprintln!("  {}: error ({})", spec.provider.name(), e);
            }
        }
    }
}

fn check_clipboard() -> bool {
    arboard::Clipboard::new().is_ok()
}
