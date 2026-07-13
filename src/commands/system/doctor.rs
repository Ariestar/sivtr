use anyhow::Result;
use serde::Serialize;
use sivtr_core::config::SivtrConfig;
use sivtr_core::workspace;
use std::path::{Path, PathBuf};

use crate::cli::DoctorArgs;
use crate::commands::interactive;
use crate::output;

pub fn execute(args: DoctorArgs) -> Result<()> {
    if args.fix && !args.json {
        return execute_interactive_fix();
    }

    let mut report = Report::default();
    report.run_checks(args.fix);

    if args.json {
        print_json(&report);
    } else {
        print_human(&report);
    }
    Ok(())
}

fn execute_interactive_fix() -> Result<()> {
    let mut report = Report::default();
    report.run_checks(false);

    print_human(&report);

    let fixable: Vec<&Check> = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Fail)
        .collect();

    if fixable.is_empty() {
        output::blank();
        output::success("nothing to fix");
        return Ok(());
    }

    output::blank();
    output::info(format!(
        "{} issue(s) can be fixed automatically",
        fixable.len()
    ));
    for check in &fixable {
        output::detail(check.label, &check.detail);
    }

    if !interactive::confirm("Fix all issues?", true)? {
        output::plain("skipped fixes");
        return Ok(());
    }

    output::blank();
    let mut fixed_report = Report::default();
    fixed_report.run_checks(true);
    print_human(&fixed_report);
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    Pass,
    Fail,
    Fixed,
    Manual,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub label: &'static str,
    pub status: Status,
    pub detail: String,
    pub hint: Option<String>,
}

impl Report {
    fn run_checks(&mut self, fix: bool) {
        self.check_binary();
        self.check_config(fix);
        self.check_session_dir();
        self.check_shell_hooks(fix);
        self.check_workspace_keys(fix);
        self.check_mcp_registration(fix);
        self.check_providers();
        self.check_clipboard();
    }

    fn add(&mut self, check: Check) {
        self.checks.push(check);
    }

    fn check_binary(&mut self) {
        self.add(Check {
            name: "binary",
            label: "binary version",
            status: Status::Pass,
            detail: format!("sivtr {}", env!("CARGO_PKG_VERSION")),
            hint: None,
        });
    }

    fn check_config(&mut self, fix: bool) {
        let path = config_path();
        if path.exists() {
            self.add(Check {
                name: "config",
                label: "config file",
                status: Status::Pass,
                detail: path.display().to_string(),
                hint: None,
            });
        } else if fix {
            match SivtrConfig::init_default() {
                Ok(created) => self.add(Check {
                    name: "config",
                    label: "config file",
                    status: Status::Fixed,
                    detail: format!("created {}", created.display()),
                    hint: None,
                }),
                Err(e) => self.add(Check {
                    name: "config",
                    label: "config file",
                    status: Status::Manual,
                    detail: format!("failed to create: {e}"),
                    hint: Some("run `sivtr config init`".to_string()),
                }),
            }
        } else {
            self.add(Check {
                name: "config",
                label: "config file",
                status: Status::Fail,
                detail: "missing".to_string(),
                hint: Some("run `sivtr config init`".to_string()),
            });
        }
    }

    fn check_session_dir(&mut self) {
        let path = session_dir();
        if let Some(path) = path {
            if path.exists() {
                let count = std::fs::read_dir(&path).map(|d| d.count()).unwrap_or(0);
                self.add(Check {
                    name: "session_dir",
                    label: "session log directory",
                    status: Status::Pass,
                    detail: format!("{} ({count} entries)", path.display()),
                    hint: None,
                });
            } else {
                self.add(Check {
                    name: "session_dir",
                    label: "session log directory",
                    status: Status::Manual,
                    detail: "missing".to_string(),
                    hint: Some(
                        "run `sivtr init <shell>`, restart the shell, then run a command"
                            .to_string(),
                    ),
                });
            }
        } else {
            self.add(Check {
                name: "session_dir",
                label: "session log directory",
                status: Status::Manual,
                detail: "unable to determine state directory".to_string(),
                hint: None,
            });
        }
    }

    fn check_shell_hooks(&mut self, fix: bool) {
        let installed = detect_installed_shells();
        if !installed.is_empty() {
            self.add(Check {
                name: "shell_hooks",
                label: "shell hooks",
                status: Status::Pass,
                detail: installed.join(", "),
                hint: None,
            });
        } else if fix {
            let shell = detect_current_shell();
            match crate::commands::capture::init::execute(&shell) {
                Ok(()) => self.add(Check {
                    name: "shell_hooks",
                    label: "shell hooks",
                    status: Status::Fixed,
                    detail: format!("installed for {shell}"),
                    hint: Some("restart your shell for hooks to take effect".to_string()),
                }),
                Err(e) => self.add(Check {
                    name: "shell_hooks",
                    label: "shell hooks",
                    status: Status::Manual,
                    detail: format!("auto-install failed: {e}"),
                    hint: Some("run `sivtr init <shell>`".to_string()),
                }),
            }
        } else {
            self.add(Check {
                name: "shell_hooks",
                label: "shell hooks",
                status: Status::Fail,
                detail: "none installed".to_string(),
                hint: Some(
                    "run `sivtr init bash` or `sivtr init zsh|powershell|nushell`".to_string(),
                ),
            });
        }
    }

    fn check_workspace_keys(&mut self, fix: bool) {
        let result = if fix {
            workspace::migrate_workspace_keys()
        } else {
            workspace::inspect_workspace_keys()
        };
        match result {
            Ok(report) => {
                if report.migrated.is_empty() && report.skipped.is_empty() {
                    self.add(Check {
                        name: "workspace_keys",
                        label: "workspace keys",
                        status: Status::Pass,
                        detail: format!("{} workspace(s) on current scheme", report.current),
                        hint: None,
                    });
                } else if fix && !report.migrated.is_empty() {
                    self.add(Check {
                        name: "workspace_keys",
                        label: "workspace keys",
                        status: Status::Fixed,
                        detail: format!("migrated {} workspace(s)", report.migrated.len()),
                        hint: None,
                    });
                } else if !report.migrated.is_empty() {
                    self.add(Check {
                        name: "workspace_keys",
                        label: "workspace keys",
                        status: Status::Fail,
                        detail: format!("{} workspace(s) need migration", report.migrated.len()),
                        hint: Some("run `sivtr doctor --fix`".to_string()),
                    });
                } else {
                    self.add(Check {
                        name: "workspace_keys",
                        label: "workspace keys",
                        status: Status::Manual,
                        detail: format!(
                            "{} workspace(s) skipped during migration",
                            report.skipped.len()
                        ),
                        hint: None,
                    });
                }
            }
            Err(e) => self.add(Check {
                name: "workspace_keys",
                label: "workspace keys",
                status: Status::Manual,
                detail: format!("migration check failed: {e}"),
                hint: None,
            }),
        }
    }

    fn check_mcp_registration(&mut self, fix: bool) {
        let targets = crate::commands::system::mcp::detect_targets();
        let registered: Vec<&str> = targets.iter().map(|p| p.name()).collect();
        if !registered.is_empty() {
            self.add(Check {
                name: "mcp",
                label: "MCP registration",
                status: Status::Pass,
                detail: format!("registered for {}", registered.join(", ")),
                hint: None,
            });
        } else if fix {
            let mcp_args = crate::cli::McpInstallArgs {
                target: "auto".to_string(),
                location: crate::cli::McpLocation::Global,
                yes: true,
            };
            match crate::commands::system::mcp::install(&mcp_args) {
                Ok(()) => self.add(Check {
                    name: "mcp",
                    label: "MCP registration",
                    status: Status::Fixed,
                    detail: "installed for detected hosts".to_string(),
                    hint: None,
                }),
                Err(e) => self.add(Check {
                    name: "mcp",
                    label: "MCP registration",
                    status: Status::Manual,
                    detail: format!("auto-install failed: {e}"),
                    hint: Some("run `sivtr mcp install auto`".to_string()),
                }),
            }
        } else {
            self.add(Check {
                name: "mcp",
                label: "MCP registration",
                status: Status::Fail,
                detail: "not registered for any host".to_string(),
                hint: Some("run `sivtr mcp install auto`".to_string()),
            });
        }
    }

    fn check_providers(&mut self) {
        let mut detail = String::new();
        for spec in sivtr_core::ai::AgentProvider::all() {
            let provider = spec.provider.session_provider();
            match provider.list_recent_sessions(None) {
                Ok(s) if s.is_empty() => {
                    detail.push_str(&format!("{}: 0  ", spec.provider.name()));
                }
                Ok(s) => {
                    detail.push_str(&format!("{}: {}  ", spec.provider.name(), s.len()));
                }
                Err(_) => {
                    detail.push_str(&format!("{}: error  ", spec.provider.name()));
                }
            }
        }
        self.add(Check {
            name: "providers",
            label: "provider sessions",
            status: Status::Pass,
            detail: detail.trim().to_string(),
            hint: None,
        });
    }

    fn check_clipboard(&mut self) {
        if arboard::Clipboard::new().is_ok() {
            self.add(Check {
                name: "clipboard",
                label: "clipboard access",
                status: Status::Pass,
                detail: "available".to_string(),
                hint: None,
            });
        } else {
            self.add(Check {
                name: "clipboard",
                label: "clipboard access",
                status: Status::Manual,
                detail: "unavailable".to_string(),
                hint: Some("copy commands may not work in this environment".to_string()),
            });
        }
    }
}

fn print_human(report: &Report) {
    let total = report.checks.len();
    let mut passed = 0;
    let mut fixed = 0;
    for check in &report.checks {
        output::info(format!("checking {}", check.label));
        match check.status {
            Status::Pass => {
                output::detail("ok", &check.detail);
                passed += 1;
            }
            Status::Fixed => {
                output::success(&check.detail);
                fixed += 1;
            }
            Status::Fail => {
                output::warning(&check.detail);
                if let Some(hint) = &check.hint {
                    output::hint(hint);
                }
            }
            Status::Manual => {
                output::detail("manual", &check.detail);
                if let Some(hint) = &check.hint {
                    output::hint(hint);
                }
            }
        }
    }
    output::blank();
    let ok = passed + fixed;
    if ok == total {
        output::success(format!("all {total} checks passed"));
    } else {
        output::warning(format!(
            "{ok}/{total} checks passed ({} fixed, {} manual)",
            fixed,
            total - ok - count_status(report, Status::Manual)
        ));
    }
}

fn count_status(report: &Report, status: Status) -> usize {
    report.checks.iter().filter(|c| c.status == status).count()
}

fn print_json(report: &Report) {
    let json = serde_json::to_string_pretty(report).unwrap_or_default();
    println!("{json}");
}

fn config_path() -> PathBuf {
    SivtrConfig::config_path().unwrap_or_else(|_| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sivtr")
            .join("config.toml")
    })
}

fn session_dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
        .map(|d| d.join("sivtr"))
}

pub fn detect_installed_shells() -> Vec<String> {
    let mut installed = Vec::new();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return installed,
    };

    for (name, rel_path, marker) in [
        ("bash", ".bashrc", "# >>> sivtr shell integration >>>"),
        ("zsh", ".zshrc", "# >>> sivtr shell integration >>>"),
    ] {
        let path = home.join(rel_path);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(marker) {
                    installed.push(name.to_string());
                }
            }
        }
    }

    if let Some(config_dir) = dirs::config_dir() {
        let nu_config = config_dir.join("nushell").join("config.nu");
        if nu_config.exists() {
            if let Ok(content) = std::fs::read_to_string(&nu_config) {
                if content.contains("# >>> sivtr shell integration >>>") {
                    installed.push("nushell".to_string());
                }
            }
        }
    }

    for cmd in &["pwsh", "powershell"] {
        if let Ok(out) = std::process::Command::new(cmd)
            .args(["-NoProfile", "-Command", "Write-Output $PROFILE"])
            .output()
        {
            let profile = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !profile.is_empty() {
                let path = Path::new(&profile);
                if path.exists() {
                    if let Ok(content) = std::fs::read_to_string(path) {
                        if content.contains("# >>> sivtr shell integration >>>") {
                            installed.push("powershell".to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    installed
}

pub fn detect_current_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("zsh") {
            return "zsh".to_string();
        }
        if shell.contains("bash") {
            return "bash".to_string();
        }
        if shell.contains("nu") {
            return "nushell".to_string();
        }
    }
    if cfg!(windows) {
        "powershell".to_string()
    } else {
        "bash".to_string()
    }
}
