use anyhow::{bail, Context, Result};
use sivtr_core::config::SivtrConfig;
use sivtr_core::workspace;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::{McpInstallArgs, McpLocation};
use crate::commands::interactive;
use crate::commands::system::doctor;
use crate::output;

const SKILL_PACKAGE: &str = "Ariestar/sivtr";
const SKILL_NAME: &str = "sivtr-memory";

pub fn execute() -> Result<()> {
    output::info("sivtr setup — get from zero to working in one command");
    output::blank();

    let shells = pick_shells()?;
    let mcp_targets = pick_mcp_targets()?;
    let install_skill = want_skill_install()?;

    run_step("initializing config", || {
        let path = SivtrConfig::init_default()?;
        Ok(format!("created {}", path.display()))
    })?;

    run_step(
        &format!("installing hooks for {}", shells.join(", ")),
        || {
            for shell in &shells {
                crate::commands::capture::init::execute(shell)?;
            }
            Ok(format!("installed for {}", shells.join(", ")))
        },
    )?;

    run_step("migrating legacy workspace keys", || {
        let report = workspace::migrate_workspace_keys()?;
        if report.migrated.is_empty() {
            Ok(format!("{} workspace(s) on current scheme", report.current))
        } else {
            Ok(format!("migrated {} workspace(s)", report.migrated.len()))
        }
    })?;

    if !mcp_targets.is_empty() {
        run_step("installing MCP for selected agent hosts", || {
            for target in &mcp_targets {
                let mcp_args = McpInstallArgs {
                    target: target.clone(),
                    location: McpLocation::Global,
                    yes: true,
                };
                crate::commands::system::mcp::install(&mcp_args)?;
            }
            Ok(format!("installed for {} host(s)", mcp_targets.len()))
        })?;
    }

    if install_skill {
        // Soft-fail: skill is recommended but should not abort hooks/MCP setup.
        match ensure_skill_installed() {
            Ok(detail) => {
                output::info("installing sivtr-memory skill");
                output::success(detail);
            }
            Err(e) => {
                output::info("installing sivtr-memory skill");
                output::warning(format!("skipped: {e}"));
                output::hint(format!(
                    "install later with `npx skills add {SKILL_PACKAGE} --skill {SKILL_NAME} -g -y`"
                ));
            }
        }
    } else {
        output::info("sivtr-memory skill");
        output::success("already installed");
    }

    run_step("running smoke test", smoke_test)?;

    output::blank();
    output::success("setup complete");
    output::blank();
    output::plain("next steps:");
    output::detail("1", "restart your shell for hooks to take effect");
    output::detail(
        "2",
        "ask your agent: fix the latest terminal error (MCP/skill path)",
    );
    output::detail("3", "run `sivtr doctor` to verify everything is healthy");
    Ok(())
}

fn pick_shells() -> Result<Vec<String>> {
    let detected = doctor::detect_current_shell();
    let installed = doctor::detect_installed_shells();
    let shells = vec![
        "powershell".to_string(),
        "bash".to_string(),
        "zsh".to_string(),
        "nushell".to_string(),
    ];
    let mut defaults = Vec::new();
    if let Some(idx) = shells.iter().position(|s| s == &detected) {
        defaults.push(idx);
    }
    for shell in &installed {
        if let Some(idx) = shells.iter().position(|s| s == shell) {
            if !defaults.contains(&idx) {
                defaults.push(idx);
            }
        }
    }
    if defaults.is_empty() {
        defaults.push(0);
    }

    if !installed.is_empty() {
        output::detail("existing hooks", installed.join(", "));
    }

    let selected = interactive::multi_select("Which shells do you use?", &shells, &defaults)?;
    if selected.is_empty() {
        return Ok(vec![detected]);
    }
    Ok(selected.into_iter().map(|i| shells[i].clone()).collect())
}

fn pick_mcp_targets() -> Result<Vec<String>> {
    let detected = crate::commands::system::mcp::detect_targets();
    let all: Vec<String> = sivtr_core::ai::AgentProvider::all()
        .iter()
        .map(|spec| spec.name.to_string())
        .collect();
    let defaults: Vec<usize> = detected
        .iter()
        .filter_map(|p| all.iter().position(|name| name == p.name()))
        .collect();

    let selected =
        interactive::multi_select("Install sivtr MCP into which agent hosts?", &all, &defaults)?;

    if selected.is_empty() {
        return Ok(Vec::new());
    }

    let targets: Vec<String> = selected
        .iter()
        .filter_map(|&i| {
            sivtr_core::ai::AgentProvider::all()
                .get(i)
                .map(|spec| spec.provider.command_name().to_string())
        })
        .collect();
    Ok(targets)
}

fn want_skill_install() -> Result<bool> {
    if skill_is_installed() {
        return Ok(false);
    }
    // Default yes: first-time setup should teach agents how to use local memory.
    interactive::confirm(
        &format!("Install the `{SKILL_NAME}` skill for agents?"),
        true,
    )
}

fn skill_is_installed() -> bool {
    skill_search_paths()
        .into_iter()
        .any(|path| path.join("SKILL.md").is_file() || path.is_dir())
}

fn skill_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".agents").join("skills").join(SKILL_NAME));
        paths.push(home.join(".claude").join("skills").join(SKILL_NAME));
        paths.push(home.join(".codex").join("skills").join(SKILL_NAME));
    }
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(user_profile);
        paths.push(home.join(".agents").join("skills").join(SKILL_NAME));
        paths.push(home.join(".claude").join("skills").join(SKILL_NAME));
    }
    paths
}

fn ensure_skill_installed() -> Result<String> {
    if skill_is_installed() {
        return Ok("already installed".to_string());
    }
    if !command_exists("npx") {
        bail!("`npx` not found; install Node.js or run the skills command manually");
    }

    let status = Command::new("npx")
        .args([
            "--yes",
            "skills",
            "add",
            SKILL_PACKAGE,
            "--skill",
            SKILL_NAME,
            "-g",
            "-y",
        ])
        .status()
        .context("failed to run `npx skills add`")?;

    if !status.success() {
        bail!("`npx skills add` exited with {status}");
    }
    if skill_is_installed() {
        Ok(format!("installed `{SKILL_NAME}` globally"))
    } else {
        // Command succeeded but we cannot see the skill dir — still treat as ok.
        Ok(format!(
            "ran install for `{SKILL_NAME}` (verify with your agent host)"
        ))
    }
}

fn command_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_step(msg: &str, action: impl FnOnce() -> Result<String>) -> Result<()> {
    output::info(msg);
    match action() {
        Ok(detail) => {
            output::success(detail);
            Ok(())
        }
        Err(e) => {
            output::warning(format!("failed: {e}"));
            Err(e)
        }
    }
}

fn smoke_test() -> Result<String> {
    let has_terminal = workspace::resolve_current_workspace()?.is_some();
    let providers: Vec<&str> = sivtr_core::ai::AgentProvider::all()
        .iter()
        .map(|spec| spec.provider.name())
        .collect();
    let skill = if skill_is_installed() {
        "installed"
    } else {
        "missing"
    };
    Ok(format!(
        "workspace: {}; providers: {}; skill: {skill}",
        if has_terminal {
            "detected"
        } else {
            "none (not in a git repo)"
        },
        providers.join(", ")
    ))
}
