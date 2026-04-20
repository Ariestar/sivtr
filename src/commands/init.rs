use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

struct HookSpec {
    hook: &'static str,
    marker_start: &'static str,
    marker_end: &'static str,
    legacy_hook: Option<&'static str>,
}

const POWERSHELL_MARKER_START: &str = "# >>> sift shell integration >>>";
const POWERSHELL_MARKER_END: &str = "# <<< sift shell integration <<<";
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

const BASH_MARKER_START: &str = "# >>> sift shell integration >>>";
const BASH_MARKER_END: &str = "# <<< sift shell integration <<<";
const BASH_HOOK: &str = r#"# >>> sift shell integration >>>
if [[ -n "${APPDATA:-}" ]]; then
  export SIFT_SESSION_LOG="${APPDATA}\\sift\\session_$$.log"
else
  export SIFT_SESSION_LOG="${XDG_STATE_HOME:-$HOME/.local/state}/sift/session_$$.log"
fi
__sift_precmd() {
  sift flush >/dev/null 2>&1 || true
}
if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  if [[ " ${PROMPT_COMMAND[*]} " != *" __sift_precmd "* ]]; then
    PROMPT_COMMAND=(__sift_precmd "${PROMPT_COMMAND[@]}")
  fi
elif [[ -n "${PROMPT_COMMAND:-}" ]]; then
  case ";$PROMPT_COMMAND;" in
    *";__sift_precmd;"*) ;;
    *) PROMPT_COMMAND="__sift_precmd;$PROMPT_COMMAND" ;;
  esac
else
  PROMPT_COMMAND="__sift_precmd"
fi
# <<< sift shell integration <<<
"#;

const ZSH_MARKER_START: &str = "# >>> sift shell integration >>>";
const ZSH_MARKER_END: &str = "# <<< sift shell integration <<<";
const ZSH_HOOK: &str = r#"# >>> sift shell integration >>>
if [[ -n "${APPDATA:-}" ]]; then
  export SIFT_SESSION_LOG="${APPDATA}\\sift\\session_$$.log"
else
  export SIFT_SESSION_LOG="${XDG_STATE_HOME:-$HOME/.local/state}/sift/session_$$.log"
fi
_sift_precmd() {
  sift flush >/dev/null 2>&1 || true
}
if (( ${precmd_functions[(I)_sift_precmd]} == 0 )); then
  precmd_functions=(_sift_precmd $precmd_functions)
fi
# <<< sift shell integration <<<
"#;

const NUSHELL_MARKER_START: &str = "# >>> sift shell integration >>>";
const NUSHELL_MARKER_END: &str = "# <<< sift shell integration <<<";
const NUSHELL_HOOK: &str = r#"# >>> sift shell integration >>>
$env.SIFT_SESSION_LOG = (($env.APPDATA? | default $nu.default-config-dir) | path join 'sift' $"session_($nu.pid).log")
if (($env.SIFT_PROMPT_WRAPPED? | default false) != true) {
    def _sift_precmd [] {
        try { ^sift flush } catch {}
    }
    $env.config.hooks.pre_prompt = ($env.config.hooks.pre_prompt? | default [] | append {|| _sift_precmd })
    $env.SIFT_PROMPT_WRAPPED = true
}
# <<< sift shell integration <<<
"#;

const POWERSHELL_SPEC: HookSpec = HookSpec {
    hook: POWERSHELL_HOOK,
    marker_start: POWERSHELL_MARKER_START,
    marker_end: POWERSHELL_MARKER_END,
    legacy_hook: Some(LEGACY_POWERSHELL_HOOK),
};

const BASH_SPEC: HookSpec = HookSpec {
    hook: BASH_HOOK,
    marker_start: BASH_MARKER_START,
    marker_end: BASH_MARKER_END,
    legacy_hook: None,
};

const ZSH_SPEC: HookSpec = HookSpec {
    hook: ZSH_HOOK,
    marker_start: ZSH_MARKER_START,
    marker_end: ZSH_MARKER_END,
    legacy_hook: None,
};

const NUSHELL_SPEC: HookSpec = HookSpec {
    hook: NUSHELL_HOOK,
    marker_start: NUSHELL_MARKER_START,
    marker_end: NUSHELL_MARKER_END,
    legacy_hook: None,
};

enum InstallStatus {
    Installed,
    Updated,
    Unchanged,
}

/// Install shell hook into the shell's profile file (one-time setup).
pub fn execute(shell: &str) -> Result<()> {
    match shell.to_lowercase().as_str() {
        "powershell" | "pwsh" => install_powershell_hook(),
        "bash" => install_single_shell_hook(&bash_profile_path()?, &BASH_SPEC),
        "zsh" => install_single_shell_hook(&zsh_profile_path()?, &ZSH_SPEC),
        "nu" | "nushell" => install_single_shell_hook(&nushell_config_path()?, &NUSHELL_SPEC),
        _ => {
            eprintln!("sift: supported shells are powershell, bash, zsh, nushell");
            eprintln!("  usage: sift init <powershell|bash|zsh|nushell>");
            std::process::exit(1);
        }
    }
}

fn install_powershell_hook() -> Result<()> {
    let mut installed = Vec::new();
    let mut updated = Vec::new();

    for cmd in &["pwsh", "powershell"] {
        if let Ok(path) = get_ps_profile(cmd) {
            match install_into_profile(Path::new(&path), &POWERSHELL_SPEC) {
                Ok(InstallStatus::Installed) => installed.push(path),
                Ok(InstallStatus::Updated) => updated.push(path),
                Ok(InstallStatus::Unchanged) => eprintln!("sift: already installed in {}", path),
                Err(_) => {}
            }
        }
    }

    print_install_summary(&installed, &updated);
    Ok(())
}

fn install_single_shell_hook(profile_path: &Path, spec: &HookSpec) -> Result<()> {
    match install_into_profile(profile_path, spec)? {
        InstallStatus::Installed => {
            eprintln!("sift: installed into {}", profile_path.display());
            eprintln!("  restart your terminal to activate");
        }
        InstallStatus::Updated => {
            eprintln!("sift: updated {}", profile_path.display());
            eprintln!("  restart your terminal to activate");
        }
        InstallStatus::Unchanged => {
            eprintln!("sift: already installed in {}", profile_path.display());
            eprintln!("sift: no new installation needed (already set up)");
        }
    }
    Ok(())
}

fn print_install_summary(installed: &[String], updated: &[String]) {
    if installed.is_empty() && updated.is_empty() {
        eprintln!("sift: no new installation needed (already set up)");
        return;
    }

    for path in installed {
        eprintln!("sift: installed into {}", path);
    }
    for path in updated {
        eprintln!("sift: updated {}", path);
    }
    eprintln!("  restart your terminal to activate");
}

fn install_into_profile(profile_path: &Path, spec: &HookSpec) -> Result<InstallStatus> {
    if profile_path.exists() {
        let content = fs::read_to_string(profile_path)?;
        if let Some(updated) = update_existing_hook(&content, spec) {
            if updated == content {
                return Ok(InstallStatus::Unchanged);
            }
            fs::write(profile_path, updated)?;
            return Ok(InstallStatus::Updated);
        }
    }

    if let Some(parent) = profile_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(profile_path)?;
    writeln!(file, "\n{}", spec.hook)?;
    Ok(InstallStatus::Installed)
}

fn bash_profile_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Failed to resolve home directory")?;
    Ok(home.join(".bashrc"))
}

fn zsh_profile_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Failed to resolve home directory")?;
    Ok(home.join(".zshrc"))
}

fn nushell_config_path() -> Result<PathBuf> {
    if let Ok(path) = get_nu_config_path("nu") {
        return Ok(PathBuf::from(path));
    }

    let config_dir = dirs::config_dir().context("Failed to resolve config directory")?;
    Ok(config_dir.join("nushell").join("config.nu"))
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

fn get_nu_config_path(cmd: &str) -> Result<String> {
    let output = Command::new(cmd)
        .args(["-c", "print $nu.config-path"])
        .output()
        .context("Failed to run shell")?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        anyhow::bail!("empty config path");
    }
    Ok(path)
}

fn update_existing_hook(content: &str, spec: &HookSpec) -> Option<String> {
    if content.contains(spec.hook) {
        return Some(content.to_string());
    }

    if let Some((start, end)) = find_marked_block(content, spec.marker_start, spec.marker_end) {
        let mut updated = String::with_capacity(content.len() - (end - start) + spec.hook.len());
        updated.push_str(&content[..start]);
        updated.push_str(spec.hook);
        updated.push_str(&content[end..]);
        return Some(updated);
    }

    if let Some(legacy_hook) = spec.legacy_hook {
        if content.contains(legacy_hook) {
            return Some(content.replacen(legacy_hook, spec.hook, 1));
        }
    }

    None
}

fn find_marked_block(content: &str, start_marker: &str, end_marker: &str) -> Option<(usize, usize)> {
    let start = content.find(start_marker)?;
    let end_marker_offset = content[start..].find(end_marker)?;
    let end = start + end_marker_offset + end_marker.len();
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::{
        update_existing_hook, BASH_HOOK, BASH_SPEC, LEGACY_POWERSHELL_HOOK, NUSHELL_HOOK,
        NUSHELL_SPEC, POWERSHELL_HOOK, POWERSHELL_SPEC, ZSH_HOOK, ZSH_SPEC,
    };

    #[test]
    fn upgrades_legacy_powershell_hook_in_place() {
        let profile = format!("before\n{}\nafter\n", LEGACY_POWERSHELL_HOOK);
        let updated = update_existing_hook(&profile, &POWERSHELL_SPEC)
            .expect("legacy hook should be detected");

        assert!(updated.contains(POWERSHELL_HOOK));
        assert!(!updated.contains(LEGACY_POWERSHELL_HOOK));
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
    }

    #[test]
    fn keeps_current_powershell_hook_unchanged() {
        let profile = format!("before\n{}\nafter\n", POWERSHELL_HOOK);
        let updated = update_existing_hook(&profile, &POWERSHELL_SPEC)
            .expect("current hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn current_powershell_hook_does_not_redirect_flush_output_handle() {
        assert!(POWERSHELL_HOOK.contains("sift flush"));
        assert!(!POWERSHELL_HOOK.contains("*>$null"));
    }

    #[test]
    fn replaces_existing_bash_block() {
        let profile = format!("before\n{}\nafter\n", BASH_HOOK);
        let updated = update_existing_hook(&profile, &BASH_SPEC)
            .expect("bash hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn replaces_existing_zsh_block() {
        let profile = format!("before\n{}\nafter\n", ZSH_HOOK);
        let updated = update_existing_hook(&profile, &ZSH_SPEC)
            .expect("zsh hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn replaces_existing_nushell_block() {
        let profile = format!("before\n{}\nafter\n", NUSHELL_HOOK);
        let updated = update_existing_hook(&profile, &NUSHELL_SPEC)
            .expect("nushell hook should be detected");

        assert_eq!(updated, profile);
    }
}
