use anyhow::Result;
use sivtr_core::config::SivtrConfig;
use sivtr_core::workspace;

use crate::cli::{McpInstallArgs, McpLocation};
use crate::commands::system::doctor;
use crate::output;

pub fn execute() -> Result<()> {
    let shell = doctor::detect_current_shell();
    let installed_shells = doctor::detect_installed_shells();

    output::info(format!("detected shell: {shell}"));
    if installed_shells.is_empty() {
        output::info("no existing shell hooks found");
    } else {
        output::detail("existing hooks", installed_shells.join(", "));
    }

    run_step("initializing config", || {
        let path = SivtrConfig::init_default()?;
        Ok(format!("created {}", path.display()))
    })?;

    run_step(&format!("installing hooks for {shell}"), || {
        crate::commands::capture::init::execute(&shell)?;
        Ok(format!("installed for {shell}"))
    })?;

    run_step("migrating legacy workspace keys", || {
        let report = workspace::migrate_workspace_keys()?;
        if report.migrated.is_empty() {
            Ok(format!("{} workspace(s) on current scheme", report.current))
        } else {
            Ok(format!("migrated {} workspace(s)", report.migrated.len()))
        }
    })?;

    run_step("installing MCP for detected agent hosts", || {
        let mcp_args = McpInstallArgs {
            target: "auto".to_string(),
            location: McpLocation::Global,
            yes: true,
        };
        crate::commands::system::mcp::install(&mcp_args)?;
        Ok("installed for detected hosts".to_string())
    })?;

    run_step("running smoke test", || smoke_test())?;

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
