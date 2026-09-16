use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct HookSpec {
    hook: &'static str,
    marker_start: &'static str,
    marker_end: &'static str,
}

const POWERSHELL_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const POWERSHELL_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const POWERSHELL_HOOK: &str = r#"# >>> sivtr shell integration >>>
if (-not $env:SIVTR_TERMINAL_ID) { $env:SIVTR_TERMINAL_ID = "$PID" }
if (-not $Global:_sivtr_prompt_wrapped) {
    $Global:_sivtr_orig_prompt = $function:prompt
    function Global:prompt {
        $rendered = if ($Global:_sivtr_orig_prompt) {
            & $Global:_sivtr_orig_prompt
        } else {
            "PS $($executionContext.SessionState.Path.CurrentLocation)> "
        }
        if ($env:SIVTR_PTY_PROXY) {
            $last = Get-History -Count 1 -ErrorAction SilentlyContinue
            if ($last) {
                $code = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
                sivtr pty-proxy report --command-id "$($last.Id)" --command "$($last.CommandLine)" --prompt "$rendered" --cwd "$($PWD.Path)" --exit $code
            }
            # PowerShell has no pre-exec hook, so the output block opens here and
            # carries the echoed command line; the proxy drops that echo.
            [Console]::Write([char]27 + "]133;C" + [char]27 + "\")
        }
        $rendered
    }
    $Global:_sivtr_prompt_wrapped = $true
}
if (-not $env:SIVTR_PTY_PROXIED -and -not $env:SIVTR_NO_PTY_PROXY -and (Get-Command sivtr -ErrorAction SilentlyContinue)) {
    $sivtrExe = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' }
    sivtr pty-proxy run $sivtrExe
    exit $LASTEXITCODE
}
# <<< sivtr shell integration <<<
"#;

const BASH_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const BASH_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const BASH_HOOK: &str = r#"# >>> sivtr shell integration >>>
export SIVTR_TERMINAL_ID="${SIVTR_TERMINAL_ID:-$$}"
__sivtr_precmd() {
  local exit_status=$?
  if [[ -n "${SIVTR_PTY_PROXY:-}" ]]; then
    local hist_entry command_id="" command=""
    hist_entry="$(HISTTIMEFORMAT= history 1)"
    if [[ $hist_entry =~ ^[[:space:]]*([0-9]+)[[:space:]]+(.*)$ ]]; then
      command_id="${BASH_REMATCH[1]}"
      command="${BASH_REMATCH[2]}"
    fi
    sivtr pty-proxy report --command-id "$command_id" --command "$command" --prompt "${PS1@P}" --cwd "$PWD" --exit "$exit_status"
  fi
  return $exit_status
}
if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  if [[ " ${PROMPT_COMMAND[*]} " != *" __sivtr_precmd "* ]]; then
    PROMPT_COMMAND=(__sivtr_precmd "${PROMPT_COMMAND[@]}")
  fi
elif [[ -n "${PROMPT_COMMAND:-}" ]]; then
  case ";$PROMPT_COMMAND;" in
    *";__sivtr_precmd;"*) ;;
    *) PROMPT_COMMAND="__sivtr_precmd;$PROMPT_COMMAND" ;;
  esac
else
  PROMPT_COMMAND="__sivtr_precmd"
fi
# Mark where a command's output starts. Only inside the proxy: with no consumer
# the marker is noise, and the `report` call above is what closes the block.
if [[ -n "${SIVTR_PTY_PROXY:-}" ]]; then
  case "${PS0:-}" in
    *$'\e]133;C'*) ;;
    *) PS0=$'\e]133;C\e\\'"${PS0:-}" ;;
  esac
fi
# Hand the terminal to the capture proxy; the profile is sourced again inside
# it and installs the hooks above. Everything below the guard is skipped there.
if [[ -z "${SIVTR_PTY_PROXIED:-}" ]] && [[ $- == *i* ]] && [[ -z "${SIVTR_NO_PTY_PROXY:-}" ]] && command -v sivtr >/dev/null 2>&1; then
  exec sivtr pty-proxy run bash
fi
# <<< sivtr shell integration <<<
"#;

const ZSH_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const ZSH_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const ZSH_HOOK: &str = r#"# >>> sivtr shell integration >>>
export SIVTR_TERMINAL_ID="${SIVTR_TERMINAL_ID:-$$}"
_sivtr_preexec() {
  printf '\033]133;C\033\\'
}
_sivtr_precmd() {
  local exit_status=$?
  if [[ -n "${SIVTR_PTY_PROXY:-}" ]]; then
    sivtr pty-proxy report --command-id "$HISTCMD" --command "$(fc -ln -1)" --prompt "$(print -P "$PROMPT")" --cwd "$PWD" --exit "$exit_status"
  fi
  return $exit_status
}
if [[ -n "${SIVTR_PTY_PROXY:-}" ]]; then
  typeset -ga preexec_functions precmd_functions
  if [[ " ${preexec_functions[*]:-} " != *" _sivtr_preexec "* ]]; then
    preexec_functions=(_sivtr_preexec $preexec_functions)
  fi
  if [[ " ${precmd_functions[*]:-} " != *" _sivtr_precmd "* ]]; then
    precmd_functions=(_sivtr_precmd $precmd_functions)
  fi
fi
if [[ -z "${SIVTR_PTY_PROXIED:-}" ]] && [[ $- == *i* ]] && [[ -z "${SIVTR_NO_PTY_PROXY:-}" ]] && (( $+commands[sivtr] )); then
  exec sivtr pty-proxy run zsh
fi
# <<< sivtr shell integration <<<
"#;

const NUSHELL_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const NUSHELL_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const NUSHELL_HOOK: &str = r#"# >>> sivtr shell integration >>>
$env.SIVTR_TERMINAL_ID = ($env.SIVTR_TERMINAL_ID? | default $"($nu.pid)")
if (($env.SIVTR_PTY_PROXY? | default "") != "") {
    # The proxy's markers are the capture boundary, so nushell's own OSC 133
    # would only add a second, unrelated set.
    $env.config.shell_integration.osc133 = false
    # `commandline` is only meaningful before the command runs, and `history` is
    # not written yet when the prompt comes back, so the command is stashed here
    # and reported below — which also keeps the metadata ahead of the `D`.
    def --env _sivtr_pre_execution [] {
        $env.SIVTR_COMMAND = (commandline)
        print -n "\e]133;C\e\\"
    }
    def --env _sivtr_pre_prompt [] {
        let code = (($env.LAST_EXIT_CODE? | default 0) | into int)
        ^sivtr pty-proxy report --command-id (random uuid) --command ($env.SIVTR_COMMAND? | default "") --prompt "" --cwd $"(pwd)" --exit $code
    }
    $env.config.hooks.pre_execution = (($env.config.hooks.pre_execution? | default []) | append {|| _sivtr_pre_execution })
    $env.config.hooks.pre_prompt = (($env.config.hooks.pre_prompt? | default []) | append {|| _sivtr_pre_prompt })
}
if (($env.SIVTR_PTY_PROXIED? | default "") == "") and (($env.SIVTR_NO_PTY_PROXY? | default "") == "") and (which sivtr | is-not-empty) {
    exec sivtr pty-proxy run nu
}
# <<< sivtr shell integration <<<
"#;

#[cfg(unix)]
const TMUX_MARKER_START: &str = "# >>> sivtr tmux shortcut >>>";
#[cfg(unix)]
const TMUX_MARKER_END: &str = "# <<< sivtr tmux shortcut <<<";
#[cfg(unix)]
const TMUX_HOOK: &str = r##"# >>> sivtr tmux shortcut >>>
bind-key y new-window -c "#{pane_current_path}" "sivtr hotkey-pick-agent --cwd . --provider all"
# <<< sivtr tmux shortcut <<<
"##;

const POWERSHELL_SPEC: HookSpec = HookSpec {
    hook: POWERSHELL_HOOK,
    marker_start: POWERSHELL_MARKER_START,
    marker_end: POWERSHELL_MARKER_END,
};

const BASH_SPEC: HookSpec = HookSpec {
    hook: BASH_HOOK,
    marker_start: BASH_MARKER_START,
    marker_end: BASH_MARKER_END,
};

const ZSH_SPEC: HookSpec = HookSpec {
    hook: ZSH_HOOK,
    marker_start: ZSH_MARKER_START,
    marker_end: ZSH_MARKER_END,
};

const NUSHELL_SPEC: HookSpec = HookSpec {
    hook: NUSHELL_HOOK,
    marker_start: NUSHELL_MARKER_START,
    marker_end: NUSHELL_MARKER_END,
};

#[cfg(unix)]
const TMUX_SPEC: HookSpec = HookSpec {
    hook: TMUX_HOOK,
    marker_start: TMUX_MARKER_START,
    marker_end: TMUX_MARKER_END,
};

#[cfg_attr(not(unix), allow(dead_code))]
const MACOS_SHORTCUT_LABEL: &str = "dev.sivtr.pick-codex";

enum InstallStatus {
    Installed,
    Updated,
    Unchanged,
}

/// Install shell hook, show status, or uninstall hooks.
pub fn execute(shell: &str) -> Result<()> {
    let target = shell.to_lowercase();
    match target.as_str() {
        "powershell" | "pwsh" => install_powershell_hook(),
        "bash" => install_single_shell_hook(&bash_profile_path()?, &BASH_SPEC),
        "zsh" => install_single_shell_hook(&zsh_profile_path()?, &ZSH_SPEC),
        "nu" | "nushell" => install_single_shell_hook(&nushell_config_path()?, &NUSHELL_SPEC),
        "all" | "-all" | "--all" => install_all_shell_hooks(),
        "tmux" => install_tmux_shortcut(),
        "linux-shortcut" => install_linux_shortcut(),
        "macos-shortcut" => install_macos_shortcut(),
        "show" | "status" => {
            show_status()?;
            Ok(())
        }
        "uninstall" => {
            uninstall_all()?;
            Ok(())
        }
        _ => {
            eprintln!(
                "sivtr: supported targets are powershell, bash, zsh, nushell, all, tmux, linux-shortcut, macos-shortcut, show, uninstall"
            );
            eprintln!(
                "  usage: sivtr init <powershell|bash|zsh|nushell|all|tmux|linux-shortcut|macos-shortcut|show|uninstall>"
            );
            std::process::exit(1);
        }
    }
}

fn install_powershell_hook() -> Result<()> {
    let mut installed = Vec::new();
    let mut updated = Vec::new();
    let mut failed = Vec::new();

    for cmd in &["pwsh", "powershell"] {
        if let Ok(path) = get_ps_profile(cmd) {
            match install_into_profile(Path::new(&path), &POWERSHELL_SPEC) {
                Ok(InstallStatus::Installed) => installed.push(path),
                Ok(InstallStatus::Updated) => updated.push(path),
                Ok(InstallStatus::Unchanged) => eprintln!("sivtr: already installed in {path}"),
                Err(err) => failed.push((path, err)),
            }
        }
    }

    print_install_summary(&installed, &updated);
    for (path, err) in failed {
        eprintln!("sivtr: failed to update {path}");
        eprintln!("  {err}");
    }
    Ok(())
}

fn install_all_shell_hooks() -> Result<()> {
    install_powershell_hook()?;
    install_single_shell_hook(&bash_profile_path()?, &BASH_SPEC)?;
    install_single_shell_hook(&zsh_profile_path()?, &ZSH_SPEC)?;
    install_single_shell_hook(&nushell_config_path()?, &NUSHELL_SPEC)?;
    Ok(())
}

fn install_single_shell_hook(profile_path: &Path, spec: &HookSpec) -> Result<()> {
    match install_into_profile(profile_path, spec)? {
        InstallStatus::Installed => {
            eprintln!("sivtr: installed into {}", profile_path.display());
            eprintln!("  restart your terminal to activate");
        }
        InstallStatus::Updated => {
            eprintln!("sivtr: updated {}", profile_path.display());
            eprintln!("  restart your terminal to activate");
        }
        InstallStatus::Unchanged => {
            eprintln!("sivtr: already installed in {}", profile_path.display());
            eprintln!("sivtr: no new installation needed (already set up)");
        }
    }
    Ok(())
}

#[cfg(unix)]
fn install_tmux_shortcut() -> Result<()> {
    let path = tmux_config_path()?;
    match install_into_profile(&path, &TMUX_SPEC)? {
        InstallStatus::Installed => {
            eprintln!("sivtr: installed tmux shortcut into {}", path.display());
            eprintln!("  shortcut: prefix + y");
            eprintln!("  reload with: tmux source-file {}", path.display());
        }
        InstallStatus::Updated => {
            eprintln!("sivtr: updated tmux shortcut in {}", path.display());
            eprintln!("  shortcut: prefix + y");
            eprintln!("  reload with: tmux source-file {}", path.display());
        }
        InstallStatus::Unchanged => {
            eprintln!(
                "sivtr: tmux shortcut already installed in {}",
                path.display()
            );
            eprintln!("  shortcut: prefix + y");
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_tmux_shortcut() -> Result<()> {
    anyhow::bail!("`sivtr init tmux` is only supported on Unix-like systems");
}

#[cfg(unix)]
fn install_linux_shortcut() -> Result<()> {
    let home = dirs::home_dir().context("Failed to resolve home directory")?;
    let bin_dir = home.join(".local").join("bin");
    let applications_dir = home.join(".local").join("share").join("applications");
    fs::create_dir_all(&bin_dir)?;
    fs::create_dir_all(&applications_dir)?;

    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let sivtr_bin = std::env::current_exe().context("Failed to resolve current executable")?;
    let terminal = detect_linux_terminal();
    let script_path = bin_dir.join("sivtr-pick-codex");
    let desktop_path = applications_dir.join("sivtr-pick-codex.desktop");

    write_linux_shortcut_script(&script_path, &cwd, &sivtr_bin, terminal.as_deref())?;
    write_linux_shortcut_desktop_entry(&desktop_path, &script_path)?;

    eprintln!("sivtr: installed Linux shortcut launcher");
    eprintln!("  script:  {}", script_path.display());
    eprintln!("  desktop: {}", desktop_path.display());
    if let Some(terminal) = terminal {
        eprintln!("  terminal: {terminal}");
    } else {
        eprintln!(
            "  terminal: not auto-detected; edit the script before binding a desktop shortcut"
        );
    }
    eprintln!("  bind your desktop shortcut to: {}", script_path.display());
    Ok(())
}

#[cfg(not(unix))]
fn install_linux_shortcut() -> Result<()> {
    anyhow::bail!("`sivtr init linux-shortcut` is only supported on Unix-like systems");
}

#[cfg(unix)]
fn install_macos_shortcut() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(anyhow::anyhow!(
            "`sivtr init macos-shortcut` must be run on macOS"
        ));
    }

    let home = dirs::home_dir().context("Failed to resolve home directory")?;
    let bin_dir = home.join(".local").join("bin");
    let launch_agents_dir = home.join("Library").join("LaunchAgents");
    fs::create_dir_all(&bin_dir)?;
    fs::create_dir_all(&launch_agents_dir)?;

    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let sivtr_bin = std::env::current_exe().context("Failed to resolve current executable")?;
    let script_path = bin_dir.join("sivtr-pick-codex");
    let plist_path = launch_agents_dir.join(format!("{MACOS_SHORTCUT_LABEL}.plist"));

    write_macos_shortcut_script(&script_path, &cwd, &sivtr_bin)?;
    write_macos_shortcut_plist(&plist_path, &script_path)?;

    eprintln!("sivtr: installed macOS shortcut launcher");
    eprintln!("  script: {}", script_path.display());
    eprintln!("  agent:  {}", plist_path.display());
    eprintln!(
        "  load with: launchctl bootstrap gui/$(id -u) {}",
        plist_path.display()
    );
    eprintln!(
        "  run manually: osascript -e 'tell application \"Terminal\" to do script \"{}\"'",
        shell_double_quote(&script_path.to_string_lossy())
    );
    Ok(())
}

#[cfg(not(unix))]
fn install_macos_shortcut() -> Result<()> {
    anyhow::bail!("`sivtr init macos-shortcut` is only supported on macOS");
}

fn print_install_summary(installed: &[String], updated: &[String]) {
    if installed.is_empty() && updated.is_empty() {
        eprintln!("sivtr: no new installation needed (already set up)");
        return;
    }

    for path in installed {
        eprintln!("sivtr: installed into {path}");
    }
    for path in updated {
        eprintln!("sivtr: updated {path}");
    }
    eprintln!("  restart your terminal to activate");
}

fn show_status() -> Result<()> {
    let mut any_installed = false;

    // PowerShell (dynamic discovery — try both pwsh and powershell)
    for cmd in &["pwsh", "powershell"] {
        if let Ok(profile) = get_ps_profile(cmd) {
            let path = Path::new(&profile);
            if path.exists() {
                let content = fs::read_to_string(path).unwrap_or_default();
                if content.contains(POWERSHELL_MARKER_START) || content.contains(POWERSHELL_HOOK) {
                    eprintln!("  powershell ({cmd}): installed in {profile}");
                    any_installed = true;
                } else {
                    eprintln!("  powershell ({cmd}): not installed ({profile})");
                }
            } else {
                eprintln!("  powershell ({cmd}): not installed (no profile at {profile})");
            }
        }
    }

    for spec_ref in shell_specs() {
        match (spec_ref.path_fn)() {
            Ok(path) => {
                if path.exists() {
                    let content = fs::read_to_string(&path).unwrap_or_default();
                    if content.contains(spec_ref.spec.marker_start)
                        || content.contains(spec_ref.spec.hook)
                    {
                        eprintln!("  {}: installed in {}", spec_ref.name, path.display());
                        any_installed = true;
                    } else {
                        eprintln!("  {}: not installed ({})", spec_ref.name, path.display());
                    }
                } else {
                    eprintln!(
                        "  {}: not installed (no profile at {})",
                        spec_ref.name,
                        path.display()
                    );
                }
            }
            Err(e) => {
                eprintln!("  {}: unavailable ({e})", spec_ref.name);
            }
        }
    }

    #[cfg(unix)]
    {
        let tmux_path = tmux_config_path()?;
        if tmux_path.exists() {
            let content = fs::read_to_string(&tmux_path).unwrap_or_default();
            if content.contains(TMUX_MARKER_START) {
                eprintln!("  tmux: shortcut installed ({})", tmux_path.display());
                any_installed = true;
            } else {
                eprintln!("  tmux: not installed");
            }
        } else {
            eprintln!("  tmux: not installed (no config)");
        }
    }

    if any_installed {
        eprintln!("  session log dir: {}", session_log_dir().display());
    } else {
        eprintln!("  no shell hooks installed");
        eprintln!("  run `sivtr init bash` (or zsh/pwsh/nushell) to install");
    }

    Ok(())
}

fn uninstall_all() -> Result<()> {
    let mut removed = 0usize;

    // PowerShell profiles
    for cmd in &["pwsh", "powershell"] {
        if let Ok(profile) = get_ps_profile(cmd) {
            let path = Path::new(&profile);
            if path.exists() {
                if let Ok(content) = fs::read_to_string(path) {
                    if let Some(updated) = remove_hook_block(&content, &POWERSHELL_SPEC) {
                        fs::write(path, updated)?;
                        eprintln!("sivtr: removed powershell ({cmd}) hook from {profile}");
                        removed += 1;
                    }
                }
            }
        }
    }

    for spec_ref in shell_specs() {
        if let Ok(path) = (spec_ref.path_fn)() {
            if path.exists() {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                if let Some(updated) = remove_hook_block(&content, spec_ref.spec) {
                    fs::write(&path, updated)
                        .with_context(|| format!("Failed to write {}", path.display()))?;
                    eprintln!(
                        "sivtr: removed {} hook from {}",
                        spec_ref.name,
                        path.display()
                    );
                    removed += 1;
                }
            }
        }
    }

    #[cfg(unix)]
    {
        let tmux_path = tmux_config_path()?;
        if tmux_path.exists() {
            let content = fs::read_to_string(&tmux_path).unwrap_or_default();
            if let Some(updated) = remove_hook_block(&content, &TMUX_SPEC) {
                fs::write(&tmux_path, updated)?;
                eprintln!("sivtr: removed tmux shortcut from {}", tmux_path.display());
                removed += 1;
            }
        }
    }

    if removed == 0 {
        eprintln!("sivtr: no hooks found to remove");
    } else {
        eprintln!("sivtr: removed {removed} hook(s). Restart your terminal to deactivate.");
    }

    Ok(())
}

struct ShellSpecRef<'a> {
    name: &'static str,
    spec: &'a HookSpec,
    path_fn: fn() -> Result<PathBuf>,
}

fn shell_specs() -> Vec<ShellSpecRef<'static>> {
    vec![
        ShellSpecRef {
            name: "bash",
            spec: &BASH_SPEC,
            path_fn: bash_profile_path,
        },
        ShellSpecRef {
            name: "zsh",
            spec: &ZSH_SPEC,
            path_fn: zsh_profile_path,
        },
        ShellSpecRef {
            name: "nushell",
            spec: &NUSHELL_SPEC,
            path_fn: nushell_config_path,
        },
    ]
    // PowerShell detected dynamically — handled inline in show_status/uninstall if needed
}

fn session_log_dir() -> PathBuf {
    sivtr_core::workspace::home_dir().join("workspaces")
}

fn remove_hook_block(content: &str, spec: &HookSpec) -> Option<String> {
    if let Some((start, end)) = find_marked_block(content, spec.marker_start, spec.marker_end) {
        let mut updated = String::with_capacity(content.len() - (end - start));
        updated.push_str(&content[..start]);
        updated.push_str(content[end..].trim_start_matches('\n'));
        Some(updated)
    } else if content.contains(spec.hook) {
        Some(content.replacen(spec.hook, "", 1))
    } else {
        None
    }
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

#[cfg(unix)]
fn tmux_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Failed to resolve home directory")?;
    Ok(home.join(".tmux.conf"))
}

fn get_ps_profile(cmd: &str) -> Result<String> {
    let path = shell_printed_path(cmd, &["-NoProfile", "-Command", "Write-Output $PROFILE"])?;
    path.context("empty profile path")
}

fn get_nu_config_path(cmd: &str) -> Result<String> {
    let path = shell_printed_path(cmd, &["-c", "print $nu.config-path"])?;
    path.context("empty config path")
}

/// Run a shell command that prints a path (`$PROFILE`, `$nu.config-path`)
/// and return the trimmed stdout; `Ok(None)` when the shell printed nothing.
/// The single "ask a shell for a path" spelling across init/doctor/MCP.
pub(crate) fn shell_printed_path(cmd: &str, args: &[&str]) -> Result<Option<String>> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .context("Failed to run shell")?;
    if !output.status.success() {
        anyhow::bail!("shell {cmd} exited with {}", output.status);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!value.is_empty()).then_some(value))
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

    None
}

#[cfg(unix)]
fn detect_linux_terminal() -> Option<String> {
    for candidate in [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "kitty",
        "alacritty",
        "foot",
        "wezterm",
        "xterm",
    ] {
        if command_exists(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(unix)]
fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

#[cfg(unix)]
fn write_linux_shortcut_script(
    path: &Path,
    cwd: &Path,
    sivtr_bin: &Path,
    terminal: Option<&str>,
) -> Result<()> {
    let script = render_linux_shortcut_script(cwd, sivtr_bin, terminal);
    fs::write(path, script)?;
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(unix)]
fn render_linux_shortcut_script(cwd: &Path, sivtr_bin: &Path, terminal: Option<&str>) -> String {
    let cwd = shell_single_quote(&cwd.to_string_lossy());
    let sivtr_bin = shell_single_quote(&sivtr_bin.to_string_lossy());
    let launcher = terminal
        .map(build_terminal_launch_command)
        .unwrap_or_else(|| {
            "printf 'Edit this launcher to choose a terminal, then run again.\\n'; read -r _"
                .to_string()
        });

    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport PROJECT_CWD='{cwd}'\nexport SIVTR_BIN='{sivtr_bin}'\n{launcher}\n"
    )
}

#[cfg(unix)]
fn build_terminal_launch_command(terminal: &str) -> String {
    let picker =
        "cd \"$PROJECT_CWD\"; exec \"$SIVTR_BIN\" hotkey-pick-agent --cwd \"$PROJECT_CWD\" --provider all";
    match terminal {
        "gnome-terminal" => format!("exec gnome-terminal -- bash -lc '{picker}'"),
        "konsole" => format!("exec konsole --noclose -e bash -lc '{picker}'"),
        "kitty" => format!("exec kitty bash -lc '{picker}'"),
        "alacritty" => format!("exec alacritty -e bash -lc '{picker}'"),
        "foot" => format!("exec foot bash -lc '{picker}'"),
        "wezterm" => format!("exec wezterm start --cwd \"$PROJECT_CWD\" -- bash -lc '{picker}'"),
        "xterm" => format!("exec xterm -e bash -lc '{picker}'"),
        _ => format!("exec {terminal} -e bash -lc '{picker}'"),
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
fn write_macos_shortcut_script(path: &Path, cwd: &Path, sivtr_bin: &Path) -> Result<()> {
    let script = render_macos_shortcut_script(cwd, sivtr_bin);
    fs::write(path, script)?;
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn render_macos_shortcut_script(cwd: &Path, sivtr_bin: &Path) -> String {
    let cwd = shell_single_quote(&cwd.to_string_lossy());
    let sivtr_bin = shell_single_quote(&sivtr_bin.to_string_lossy());
    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport PROJECT_CWD='{cwd}'\nexport SIVTR_BIN='{sivtr_bin}'\ncd \"$PROJECT_CWD\"\nexec \"$SIVTR_BIN\" hotkey-pick-agent --cwd \"$PROJECT_CWD\" --provider all\n"
    )
}

#[cfg_attr(not(unix), allow(dead_code))]
fn write_macos_shortcut_plist(path: &Path, script_path: &Path) -> Result<()> {
    let plist = render_macos_shortcut_plist(script_path);
    fs::write(path, plist)?;
    Ok(())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn render_macos_shortcut_plist(script_path: &Path) -> String {
    let script = xml_escape(&script_path.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{MACOS_SHORTCUT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/osascript</string>
    <string>-e</string>
    <string>tell application "Terminal" to do script "{script}"</string>
  </array>
</dict>
</plist>
"#
    )
}

#[cfg(unix)]
fn write_linux_shortcut_desktop_entry(path: &Path, script_path: &Path) -> Result<()> {
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Sivtr Pick Codex\nExec={}\nTerminal=false\nCategories=Development;\n",
        desktop_exec_quote(&script_path.to_string_lossy())
    );
    fs::write(path, desktop)?;
    Ok(())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\"'\"'")
}

#[cfg_attr(not(unix), allow(dead_code))]
fn desktop_exec_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg_attr(not(unix), allow(dead_code))]
fn shell_double_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg_attr(not(unix), allow(dead_code))]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn find_marked_block(
    content: &str,
    start_marker: &str,
    end_marker: &str,
) -> Option<(usize, usize)> {
    let start = content.find(start_marker)?;
    let end_marker_offset = content[start..].find(end_marker)?;
    let end = start + end_marker_offset + end_marker.len();
    Some((start, end))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::{
        build_terminal_launch_command, command_exists, render_linux_shortcut_script,
        shell_single_quote, TMUX_HOOK, TMUX_SPEC,
    };
    use super::{
        desktop_exec_quote, render_macos_shortcut_plist, render_macos_shortcut_script,
        shell_printed_path, update_existing_hook, xml_escape, BASH_HOOK, BASH_SPEC,
        MACOS_SHORTCUT_LABEL, NUSHELL_HOOK, NUSHELL_SPEC, POWERSHELL_HOOK, POWERSHELL_SPEC,
        ZSH_HOOK, ZSH_SPEC,
    };
    use std::path::Path;

    #[test]
    fn shell_printed_path_rejects_nonzero_exit_code() {
        // A failed shell that still writes stdout must not yield a path.
        #[cfg(unix)]
        let result = shell_printed_path("sh", &["-c", "echo junk; exit 1"]);
        #[cfg(windows)]
        let result = shell_printed_path("cmd", &["/C", "echo junk & exit /b 1"]);
        assert!(result.is_err(), "nonzero exit must not parse as a path");
    }

    #[test]
    fn keeps_current_powershell_hook_unchanged() {
        let profile = format!("before\n{POWERSHELL_HOOK}\nafter\n");
        let updated = update_existing_hook(&profile, &POWERSHELL_SPEC)
            .expect("current hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn every_hook_proxies_the_terminal_and_gates_capture() {
        for hook in [POWERSHELL_HOOK, BASH_HOOK, ZSH_HOOK, NUSHELL_HOOK] {
            // One re-exec entry, and it must be guarded or the shell loops.
            assert!(hook.contains("sivtr pty-proxy run"), "{hook}");
            assert!(hook.contains("SIVTR_PTY_PROXIED"), "{hook}");
            // Capture is only announced when a proxy is actually listening.
            assert!(hook.contains("SIVTR_PTY_PROXY"), "{hook}");
            assert!(hook.contains("pty-proxy report"), "{hook}");
            assert!(!hook.contains("sivtr flush"), "{hook}");
            // The removed tee/trap approach must not come back.
            assert!(!hook.contains("tee \"$SIVTR_CAPTURE_FILE\""), "{hook}");
        }

        // Every hook opens the block itself and lets `report` close it.
        for hook in [POWERSHELL_HOOK, BASH_HOOK, ZSH_HOOK, NUSHELL_HOOK] {
            assert!(hook.contains("133;C"), "{hook}");
        }
    }

    #[test]
    fn shell_hooks_do_not_run_workspace_resolution() {
        for hook in [POWERSHELL_HOOK, BASH_HOOK, ZSH_HOOK, NUSHELL_HOOK] {
            assert!(!hook.contains("git rev-parse"));
            assert!(!hook.contains("terminal-log"));
            assert!(!hook.contains("SIVTR_SESSION_LOG"));
        }
    }

    #[test]
    fn nushell_hook_disables_native_markers() {
        // Nushell ships its own OSC 133 integration; ours is the capture
        // boundary, so the native one must be turned off to avoid a second set.
        assert!(NUSHELL_HOOK.contains("shell_integration.osc133 = false"));
        assert!(NUSHELL_HOOK.contains("hooks.pre_execution"));
        assert!(NUSHELL_HOOK.contains("hooks.pre_prompt"));
    }

    #[test]
    fn replaces_existing_bash_block() {
        let profile = format!("before\n{BASH_HOOK}\nafter\n");
        let updated =
            update_existing_hook(&profile, &BASH_SPEC).expect("bash hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn bash_hook_preserves_the_session_id_and_a_user_ps0() {
        // The proxy sets SIVTR_TERMINAL_ID for the whole session; overwriting it
        // here would split one terminal across two session logs.
        assert!(BASH_HOOK.contains(r#"export SIVTR_TERMINAL_ID="${SIVTR_TERMINAL_ID:-$$}""#));
        // A pre-existing PS0 is kept, and re-sourcing the profile must not stack
        // markers onto it.
        assert!(BASH_HOOK.contains(r#"PS0=$'\e]133;C\e\\'"${PS0:-}""#));
        assert!(BASH_HOOK.contains(r"*$'\e]133;C'*"));
        // The DEBUG trap is what bash-preexec needs; PS0 avoids it entirely.
        assert!(!BASH_HOOK.contains("trap '__sivtr_preexec' DEBUG"));
    }

    #[test]
    fn replaces_existing_zsh_block() {
        let profile = format!("before\n{ZSH_HOOK}\nafter\n");
        let updated =
            update_existing_hook(&profile, &ZSH_SPEC).expect("zsh hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn zsh_hook_preserves_the_session_id() {
        assert!(ZSH_HOOK.contains(r#"export SIVTR_TERMINAL_ID="${SIVTR_TERMINAL_ID:-$$}""#));
        assert!(ZSH_HOOK.contains("preexec_functions=(_sivtr_preexec"));
        assert!(ZSH_HOOK.contains("precmd_functions=(_sivtr_precmd"));
    }

    #[test]
    fn replaces_existing_nushell_block() {
        let profile = format!("before\n{NUSHELL_HOOK}\nafter\n");
        let updated =
            update_existing_hook(&profile, &NUSHELL_SPEC).expect("nushell hook should be detected");

        assert_eq!(updated, profile);
    }

    #[cfg(unix)]
    #[test]
    fn replaces_existing_tmux_block() {
        let profile = format!("before\n{TMUX_HOOK}\nafter\n");
        let updated =
            update_existing_hook(&profile, &TMUX_SPEC).expect("tmux hook should be detected");

        assert_eq!(updated, profile);
    }

    #[cfg(unix)]
    #[test]
    fn gnome_terminal_launcher_uses_project_cwd() {
        let command = build_terminal_launch_command("gnome-terminal");

        assert!(command.contains("gnome-terminal"));
        assert!(command.contains(
            "cd \"$PROJECT_CWD\"; exec \"$SIVTR_BIN\" hotkey-pick-agent --cwd \"$PROJECT_CWD\" --provider all"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn shell_single_quote_escapes_single_quotes() {
        assert_eq!(shell_single_quote("/tmp/it's"), "/tmp/it'\"'\"'s");
    }

    #[cfg(unix)]
    #[test]
    fn linux_shortcut_script_exports_project_cwd() {
        let script = render_linux_shortcut_script(
            Path::new("/tmp/project"),
            Path::new("/tmp/bin/sivtr"),
            Some("xterm"),
        );

        assert!(script.contains("export PROJECT_CWD='/tmp/project'"));
        assert!(script.contains("export SIVTR_BIN='/tmp/bin/sivtr'"));
        assert!(script.contains(
            "cd \"$PROJECT_CWD\"; exec \"$SIVTR_BIN\" hotkey-pick-agent --cwd \"$PROJECT_CWD\" --provider all"
        ));
    }

    #[test]
    fn desktop_exec_quote_wraps_paths_with_spaces() {
        assert_eq!(
            desktop_exec_quote("/tmp/sivtr desktop/sivtr-pick-codex"),
            "\"/tmp/sivtr desktop/sivtr-pick-codex\""
        );
    }

    #[test]
    fn desktop_exec_quote_escapes_embedded_quotes_and_backslashes() {
        assert_eq!(
            desktop_exec_quote("/tmp/dir \\\"quoted\\\"/sivtr"),
            "\"/tmp/dir \\\\\\\"quoted\\\\\\\"/sivtr\""
        );
    }

    #[test]
    fn macos_shortcut_script_runs_picker_in_project_cwd() {
        let script =
            render_macos_shortcut_script(Path::new("/tmp/project"), Path::new("/tmp/bin/sivtr"));

        assert!(script.contains("export PROJECT_CWD='/tmp/project'"));
        assert!(script.contains("export SIVTR_BIN='/tmp/bin/sivtr'"));
        assert!(script.contains(
            "exec \"$SIVTR_BIN\" hotkey-pick-agent --cwd \"$PROJECT_CWD\" --provider all"
        ));
    }

    #[test]
    fn macos_shortcut_plist_uses_terminal_osascript_launcher() {
        let plist = render_macos_shortcut_plist(Path::new("/tmp/sivtr-pick-codex"));

        assert!(plist.contains(MACOS_SHORTCUT_LABEL));
        assert!(plist.contains("/usr/bin/osascript"));
        assert!(plist.contains("tell application \"Terminal\" to do script"));
        assert!(plist.contains("/tmp/sivtr-pick-codex"));
    }

    #[test]
    fn xml_escape_escapes_special_characters() {
        assert_eq!(xml_escape("a&b<c>d"), "a&amp;b&lt;c&gt;d");
    }

    #[cfg(unix)]
    #[test]
    fn command_exists_detects_programs_via_path_scan() {
        assert!(command_exists("sh"));
    }
}
