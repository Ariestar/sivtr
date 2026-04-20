use anyhow::{Result, Context};
use std::fs;
use std::io::Write;
use std::process::Command;

const LEGACY_SIFT_MARKER: &str = "# sift shell integration";
const SIFT_MARKER_START: &str = "# >>> sift shell integration >>>";
const SIFT_MARKER_END: &str = "# <<< sift shell integration <<<";
const LEGACY_POWERSHELL_HOOK: &str = r#"# sift shell integration
$env:SIFT_SESSION_LOG = Join-Path $env:APPDATA "sift\session_$PID.log"
$Global:_sift_orig_prompt = $function:prompt
function Global:prompt { try { sift flush *>$null } catch {}; & $Global:_sift_orig_prompt }
"#;

const POWERSHELL_HOOK: &str = r#"# >>> sift shell integration >>>
$env:SIFT_SESSION_LOG = Join-Path $env:APPDATA "sift\session_$PID.log"
if (-not $Global:_sift_prompt_wrapped) {
    $Global:_sift_orig_prompt = $function:prompt
    function Global:prompt {
        try { sift flush } catch {}
        if ($Global:_sift_orig_prompt) {
            & $Global:_sift_orig_prompt
        } else {
            "PS $($executionContext.SessionState.Path.CurrentLocation)> "
        }
    }
    $Global:_sift_prompt_wrapped = $true
}
# <<< sift shell integration <<<
"#;

enum InstallStatus {
    Installed,
    Updated,
    Unchanged,
}

/// Install shell hook into the shell's profile file (one-time setup).
pub fn execute(shell: &str) -> Result<()> {
    match shell.to_lowercase().as_str() {
        "powershell" | "pwsh" => install_powershell_hook(),
        _ => {
            eprintln!("sift: currently only PowerShell is supported");
            eprintln!("  usage: sift init powershell");
            std::process::exit(1);
        }
    }
}

fn install_powershell_hook() -> Result<()> {
    let mut installed = Vec::new();
    let mut updated = Vec::new();

    // Install into all available PowerShell profiles
    for cmd in &["pwsh", "powershell"] {
        if let Ok(path) = get_ps_profile(cmd) {
            match install_into_profile(&path) {
                Ok(InstallStatus::Installed) => installed.push(path),
                Ok(InstallStatus::Updated) => updated.push(path),
                Ok(InstallStatus::Unchanged) => eprintln!("sift: already installed in {}", path),
                Err(_) => {}
            }
        }
    }

    if installed.is_empty() && updated.is_empty() {
        eprintln!("sift: no new installation needed (already set up)");
    } else {
        for p in &installed {
            eprintln!("sift: installed into {}", p);
        }
        for p in &updated {
            eprintln!("sift: updated {}", p);
        }
        eprintln!("  restart your terminal to activate");
    }
    Ok(())
}

fn install_into_profile(profile_path: &str) -> Result<InstallStatus> {
    let profile = std::path::Path::new(profile_path);

    if profile.exists() {
        let content = fs::read_to_string(profile)?;
        if let Some(updated) = update_existing_hook(&content) {
            if updated == content {
                return Ok(InstallStatus::Unchanged);
            }
            fs::write(profile, updated)?;
            return Ok(InstallStatus::Updated);
        }
    }

    if let Some(parent) = profile.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(profile)?;
    writeln!(file, "\n{}", POWERSHELL_HOOK)?;
    Ok(InstallStatus::Installed)
}

fn get_ps_profile(cmd: &str) -> Result<String> {
    let output = Command::new(cmd)
        .args(["-NoProfile", "-Command", "Write-Output $PROFILE"])
        .output()
        .context("Failed to run shell")?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        anyhow::bail!("empty profile path");
    }
    Ok(path)
}

fn update_existing_hook(content: &str) -> Option<String> {
    if content.contains(POWERSHELL_HOOK) {
        return Some(content.to_string());
    }

    if let Some((start, end)) = find_marked_block(content) {
        let mut updated = String::with_capacity(content.len() - (end - start) + POWERSHELL_HOOK.len());
        updated.push_str(&content[..start]);
        updated.push_str(POWERSHELL_HOOK);
        updated.push_str(&content[end..]);
        return Some(updated);
    }

    if content.contains(LEGACY_POWERSHELL_HOOK) {
        return Some(content.replacen(LEGACY_POWERSHELL_HOOK, POWERSHELL_HOOK, 1));
    }

    if content.contains(LEGACY_SIFT_MARKER) {
        return Some(content.replacen(LEGACY_SIFT_MARKER, SIFT_MARKER_START, 1));
    }

    None
}

fn find_marked_block(content: &str) -> Option<(usize, usize)> {
    let start = content.find(SIFT_MARKER_START)?;
    let end_marker = content[start..].find(SIFT_MARKER_END)?;
    let end = start + end_marker + SIFT_MARKER_END.len();
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::{update_existing_hook, LEGACY_POWERSHELL_HOOK, POWERSHELL_HOOK};

    #[test]
    fn upgrades_legacy_hook_in_place() {
        let profile = format!("before\n{}\nafter\n", LEGACY_POWERSHELL_HOOK);
        let updated = update_existing_hook(&profile).expect("legacy hook should be detected");

        assert!(updated.contains(POWERSHELL_HOOK));
        assert!(!updated.contains(LEGACY_POWERSHELL_HOOK));
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
    }

    #[test]
    fn keeps_current_hook_unchanged() {
        let profile = format!("before\n{}\nafter\n", POWERSHELL_HOOK);
        let updated = update_existing_hook(&profile).expect("current hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn current_hook_does_not_redirect_flush_output_handle() {
        assert!(POWERSHELL_HOOK.contains("sift flush"));
        assert!(!POWERSHELL_HOOK.contains("*>$null"));
    }
}
