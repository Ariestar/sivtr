use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

pub const SKILL_PACKAGE: &str = "Ariestar/sivtr";
pub const SKILL_NAME: &str = "sivtr-memory";

/// Returns true if the bundled skill appears installed for common agent hosts.
pub fn is_installed() -> bool {
    search_paths()
        .into_iter()
        .any(|path| path.join("SKILL.md").is_file() || path.is_dir())
}

/// Paths checked for a global `sivtr-memory` install.
pub fn search_paths() -> Vec<PathBuf> {
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

/// Install the skill globally via `npx skills add` when missing.
pub fn ensure_installed() -> Result<String> {
    if is_installed() {
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
    if is_installed() {
        Ok(format!("installed `{SKILL_NAME}` globally"))
    } else {
        Ok(format!(
            "ran install for `{SKILL_NAME}` (verify with your agent host)"
        ))
    }
}

pub fn install_hint() -> String {
    format!("run `npx skills add {SKILL_PACKAGE} --skill {SKILL_NAME} -g -y`")
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
