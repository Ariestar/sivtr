use anyhow::Result;
use sivtr_core::config::SivtrConfig;
use sivtr_core::workspace;

use crate::cli::{McpInstallArgs, McpLocation};
use crate::commands::interactive;
use crate::commands::system::doctor;
use crate::output;

pub fn execute() -> Result<()> {
    output::info("sivtr setup — get from zero to working in one command");
    output::blank();

    let shells = pick_shells()?;
    let mcp_targets = pick_mcp_targets()?;

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

    run_step("running smoke test", smoke_test)?;

    output::blank();
    output::success("setup complete");
    output::blank();
    output::plain("next steps:");
    output::detail("1", "restart your shell for hooks to take effect");
    output::detail(
        "2",
        "run a command, then `sivtr search terminal --latest 5`",
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

    let selected = interactive::multi_select(
        "Install sivtr MCP into which agent hosts?",
        &all,
        &defaults,
    )?;

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
    Ok(format!(
        "workspace: {}; providers: {}",
        if has_terminal {
            "detected"
        } else {
            "none (not in a git repo)"
        },
        providers.join(", ")
    ))
}
