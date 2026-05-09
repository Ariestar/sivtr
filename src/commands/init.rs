use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(windows)]
use crate::cli::{HotkeyAction, HotkeyCommand, HotkeyStartArgs};
#[cfg(windows)]
use std::io;

struct HookSpec {
    hook: &'static str,
    marker_start: &'static str,
    marker_end: &'static str,
    legacy_marker_start: &'static str,
    legacy_marker_end: &'static str,
    legacy_hook: Option<&'static str>,
}

const LEGACY_MARKER_START: &str = "# >>> sift shell integration >>>";
const LEGACY_MARKER_END: &str = "# <<< sift shell integration <<<";
const POWERSHELL_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const POWERSHELL_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const LEGACY_POWERSHELL_HOOK: &str = r#"# sift shell integration
$env:SIFT_SESSION_LOG = Join-Path $env:APPDATA "sift\session_$PID.log"
$Global:_sift_orig_prompt = $function:prompt
function Global:prompt { try { sift flush *>$null } catch {}; & $Global:_sift_orig_prompt }
"#;
const POWERSHELL_HOOK: &str = r#"# >>> sivtr shell integration >>>
$env:SIVTR_SESSION_LOG = Join-Path $env:APPDATA "sivtr\session_$PID.log"
if (-not $Global:_sivtr_prompt_wrapped) {
    $Global:_sivtr_orig_prompt = $function:prompt
    function Global:prompt {
        $rendered = if ($Global:_sivtr_orig_prompt) {
            & $Global:_sivtr_orig_prompt
        } else {
            "PS $($executionContext.SessionState.Path.CurrentLocation)> "
        }
        try {
            $last = Get-History -Count 1 -ErrorAction SilentlyContinue
            if ($last) {
                $env:SIVTR_LAST_COMMAND = [string]$last.CommandLine
                $env:SIVTR_LAST_COMMAND_ID = [string]$last.Id
            }
            $env:SIVTR_LAST_PROMPT = [string]$rendered
            sivtr flush
        } catch {}
        $rendered
    }
    $Global:_sivtr_prompt_wrapped = $true
}
# <<< sivtr shell integration <<<
"#;

const BASH_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const BASH_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const BASH_HOOK: &str = r#"# >>> sivtr shell integration >>>
if [[ -n "${APPDATA:-}" ]]; then
  export SIVTR_SESSION_LOG="${APPDATA}\\sivtr\\session_$$.log"
else
  export SIVTR_SESSION_LOG="${XDG_STATE_HOME:-$HOME/.local/state}/sivtr/session_$$.log"
fi
export SIVTR_CAPTURE_FILE="${SIVTR_SESSION_LOG%.log}.capture"
mkdir -p "${SIVTR_SESSION_LOG%/*}"
__sivtr_capture_active=0
__sivtr_prompt_phase=0
__sivtr_restore_capture() {
  if [[ "${__sivtr_capture_active:-0}" -eq 1 ]]; then
    exec 1>&19 2>&20
    exec 19>&- 20>&-
    __sivtr_capture_active=0
  fi
}
__sivtr_preexec() {
  [[ "${__sivtr_prompt_phase:-0}" -eq 1 ]] && return
  [[ "${__sivtr_capture_active:-0}" -eq 1 ]] && return
  : >| "$SIVTR_CAPTURE_FILE"
  exec 19>&1 20>&2
  exec > >(tee "$SIVTR_CAPTURE_FILE" >&19) 2>&1
  __sivtr_capture_active=1
}
__sivtr_precmd() {
  local exit_status=$?
  __sivtr_prompt_phase=1
  __sivtr_restore_capture
  local hist_entry
  hist_entry="$(HISTTIMEFORMAT= history 1)"
  if [[ $hist_entry =~ ^[[:space:]]*([0-9]+)[[:space:]]+(.*)$ ]]; then
    export SIVTR_LAST_COMMAND_ID="${BASH_REMATCH[1]}"
    export SIVTR_LAST_COMMAND="${BASH_REMATCH[2]}"
  fi
  export SIVTR_LAST_PROMPT="${PS1@P}"
  sivtr flush >/dev/null 2>&1 || true
  __sivtr_prompt_phase=0
  return $exit_status
}
trap '__sivtr_preexec' DEBUG
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
# <<< sivtr shell integration <<<
"#;

const ZSH_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const ZSH_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const ZSH_HOOK: &str = r#"# >>> sivtr shell integration >>>
if [[ -n "${APPDATA:-}" ]]; then
  export SIVTR_SESSION_LOG="${APPDATA}\\sivtr\\session_$$.log"
else
  export SIVTR_SESSION_LOG="${XDG_STATE_HOME:-$HOME/.local/state}/sivtr/session_$$.log"
fi
export SIVTR_CAPTURE_FILE="${SIVTR_SESSION_LOG%.log}.capture"
mkdir -p "${SIVTR_SESSION_LOG%/*}"
typeset -ga preexec_functions precmd_functions
typeset -g __sivtr_capture_active=0
typeset -gi __sivtr_stdout_fd=-1 __sivtr_stderr_fd=-1
_sivtr_restore_capture() {
  if (( __sivtr_capture_active )); then
    exec 1>&$__sivtr_stdout_fd
    exec 2>&$__sivtr_stderr_fd
    exec {__sivtr_stdout_fd}>&-
    exec {__sivtr_stderr_fd}>&-
    __sivtr_capture_active=0
  fi
}
_sivtr_preexec() {
  (( __sivtr_capture_active )) && return
  : >| "$SIVTR_CAPTURE_FILE"
  exec {__sivtr_stdout_fd}>&1
  exec {__sivtr_stderr_fd}>&2
  exec > >(tee "$SIVTR_CAPTURE_FILE" >&$__sivtr_stdout_fd) 2>&1
  __sivtr_capture_active=1
}
_sivtr_precmd() {
  local exit_status=$?
  _sivtr_restore_capture
  export SIVTR_LAST_COMMAND="$(fc -ln -1)"
  export SIVTR_LAST_COMMAND_ID="$HISTCMD"
  export SIVTR_LAST_PROMPT="$(print -P "$PROMPT")"
  sivtr flush >/dev/null 2>&1 || true
  return $exit_status
}
if [[ " ${preexec_functions[*]:-} " != *" _sivtr_preexec "* ]]; then
  preexec_functions=(_sivtr_preexec $preexec_functions)
fi
if [[ " ${precmd_functions[*]:-} " != *" _sivtr_precmd "* ]]; then
  precmd_functions=(_sivtr_precmd $precmd_functions)
fi
# <<< sivtr shell integration <<<
"#;

const NUSHELL_MARKER_START: &str = "# >>> sivtr shell integration >>>";
const NUSHELL_MARKER_END: &str = "# <<< sivtr shell integration <<<";
const NUSHELL_HOOK: &str = r#"# >>> sivtr shell integration >>>
$env.SIVTR_SESSION_LOG = (($env.APPDATA? | default $nu.default-config-dir) | path join 'sivtr' $"session_($nu.pid).log")
if (($env.SIVTR_PROMPT_WRAPPED? | default false) != true) {
    def _sivtr_precmd [] {
        let last = (history | last 1 | get 0?)
        if $last != null {
            $env.SIVTR_LAST_COMMAND = ($last.command? | default "")
            $env.SIVTR_LAST_COMMAND_ID = (($last.start_timestamp? | default (date now)) | into string)
        }
        $env.SIVTR_LAST_PROMPT = (do $env.PROMPT_COMMAND)
        try { ^sivtr flush } catch {}
    }
    $env.config.hooks.pre_prompt = ($env.config.hooks.pre_prompt? | default [] | append {|| _sivtr_precmd })
    $env.SIVTR_PROMPT_WRAPPED = true
}
# <<< sivtr shell integration <<<
"#;

#[cfg(unix)]
const TMUX_MARKER_START: &str = "# >>> sivtr tmux shortcut >>>";
#[cfg(unix)]
const TMUX_MARKER_END: &str = "# <<< sivtr tmux shortcut <<<";
#[cfg(unix)]
const TMUX_HOOK: &str = r##"# >>> sivtr tmux shortcut >>>
bind-key y new-window -c "#{pane_current_path}" "sivtr copy codex --pick"
# <<< sivtr tmux shortcut <<<
"##;

const POWERSHELL_SPEC: HookSpec = HookSpec {
    hook: POWERSHELL_HOOK,
    marker_start: POWERSHELL_MARKER_START,
    marker_end: POWERSHELL_MARKER_END,
    legacy_marker_start: LEGACY_MARKER_START,
    legacy_marker_end: LEGACY_MARKER_END,
    legacy_hook: Some(LEGACY_POWERSHELL_HOOK),
};

const BASH_SPEC: HookSpec = HookSpec {
    hook: BASH_HOOK,
    marker_start: BASH_MARKER_START,
    marker_end: BASH_MARKER_END,
    legacy_marker_start: LEGACY_MARKER_START,
    legacy_marker_end: LEGACY_MARKER_END,
    legacy_hook: None,
};

const ZSH_SPEC: HookSpec = HookSpec {
    hook: ZSH_HOOK,
    marker_start: ZSH_MARKER_START,
    marker_end: ZSH_MARKER_END,
    legacy_marker_start: LEGACY_MARKER_START,
    legacy_marker_end: LEGACY_MARKER_END,
    legacy_hook: None,
};

const NUSHELL_SPEC: HookSpec = HookSpec {
    hook: NUSHELL_HOOK,
    marker_start: NUSHELL_MARKER_START,
    marker_end: NUSHELL_MARKER_END,
    legacy_marker_start: LEGACY_MARKER_START,
    legacy_marker_end: LEGACY_MARKER_END,
    legacy_hook: None,
};

#[cfg(unix)]
const TMUX_SPEC: HookSpec = HookSpec {
    hook: TMUX_HOOK,
    marker_start: TMUX_MARKER_START,
    marker_end: TMUX_MARKER_END,
    legacy_marker_start: TMUX_MARKER_START,
    legacy_marker_end: TMUX_MARKER_END,
    legacy_hook: None,
};

#[cfg_attr(not(unix), allow(dead_code))]
const MACOS_SHORTCUT_LABEL: &str = "dev.sivtr.pick-codex";

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
        "tmux" => install_tmux_shortcut(),
        "linux-shortcut" => install_linux_shortcut(),
        "macos-shortcut" => install_macos_shortcut(),
        _ => {
            eprintln!(
                "sivtr: supported targets are powershell, bash, zsh, nushell, tmux, linux-shortcut, macos-shortcut"
            );
            eprintln!(
                "  usage: sivtr init <powershell|bash|zsh|nushell|tmux|linux-shortcut|macos-shortcut>"
            );
            std::process::exit(1);
        }
    }?;

    maybe_start_hotkey()
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
    let terminal = detect_linux_terminal();
    let script_path = bin_dir.join("sivtr-pick-codex");
    let desktop_path = applications_dir.join("sivtr-pick-codex.desktop");

    write_linux_shortcut_script(&script_path, &cwd, terminal.as_deref())?;
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
    let script_path = bin_dir.join("sivtr-pick-codex");
    let plist_path = launch_agents_dir.join(format!("{MACOS_SHORTCUT_LABEL}.plist"));

    write_macos_shortcut_script(&script_path, &cwd)?;
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

fn maybe_start_hotkey() -> Result<()> {
    #[cfg(windows)]
    {
        if !atty::is(atty::Stream::Stdin) {
            eprintln!("sivtr: start the Codex hotkey later with `sivtr hotkey start`");
            return Ok(());
        }

        if prompt_start_hotkey()? {
            crate::commands::hotkey::execute(HotkeyCommand {
                action: Some(HotkeyAction::Start(HotkeyStartArgs { chord: None })),
            })?;
        } else {
            eprintln!("sivtr: start the Codex hotkey later with `sivtr hotkey start`");
        }
    }

    Ok(())
}

#[cfg(windows)]
fn prompt_start_hotkey() -> Result<bool> {
    eprint!("sivtr: start the Windows Codex hotkey now? [Y/n] ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(parse_yes_no_default_yes(&input))
}

#[cfg(windows)]
fn parse_yes_no_default_yes(input: &str) -> bool {
    !matches!(input.trim().to_lowercase().as_str(), "n" | "no")
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

    if let Some((start, end)) =
        find_marked_block(content, spec.legacy_marker_start, spec.legacy_marker_end)
    {
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
fn write_linux_shortcut_script(path: &Path, cwd: &Path, terminal: Option<&str>) -> Result<()> {
    let script = render_linux_shortcut_script(cwd, terminal);
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
fn render_linux_shortcut_script(cwd: &Path, terminal: Option<&str>) -> String {
    let cwd = shell_single_quote(&cwd.to_string_lossy());
    let launcher = terminal
        .map(build_terminal_launch_command)
        .unwrap_or_else(|| {
            "printf 'Edit this launcher to choose a terminal, then run again.\\n'; read -r _"
                .to_string()
        });

    format!("#!/usr/bin/env bash\nset -euo pipefail\nexport PROJECT_CWD='{cwd}'\n{launcher}\n")
}

#[cfg(unix)]
fn build_terminal_launch_command(terminal: &str) -> String {
    let picker = "cd \"$PROJECT_CWD\"; exec sivtr copy codex --pick";
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
fn write_macos_shortcut_script(path: &Path, cwd: &Path) -> Result<()> {
    let script = render_macos_shortcut_script(cwd);
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
fn render_macos_shortcut_script(cwd: &Path) -> String {
    let cwd = shell_single_quote(&cwd.to_string_lossy());
    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport PROJECT_CWD='{cwd}'\ncd \"$PROJECT_CWD\"\nexec sivtr copy codex --pick\n"
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

#[cfg(unix)]
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
        update_existing_hook, xml_escape, BASH_HOOK, BASH_SPEC, LEGACY_POWERSHELL_HOOK,
        MACOS_SHORTCUT_LABEL, NUSHELL_HOOK, NUSHELL_SPEC, POWERSHELL_HOOK, POWERSHELL_SPEC,
        ZSH_HOOK, ZSH_SPEC,
    };
    use std::path::Path;

    #[cfg(windows)]
    use super::parse_yes_no_default_yes;

    #[test]
    fn upgrades_legacy_powershell_hook_in_place() {
        let profile = format!("before\n{LEGACY_POWERSHELL_HOOK}\nafter\n");
        let updated = update_existing_hook(&profile, &POWERSHELL_SPEC)
            .expect("legacy hook should be detected");

        assert!(updated.contains(POWERSHELL_HOOK));
        assert!(!updated.contains(LEGACY_POWERSHELL_HOOK));
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
    }

    #[test]
    fn keeps_current_powershell_hook_unchanged() {
        let profile = format!("before\n{POWERSHELL_HOOK}\nafter\n");
        let updated = update_existing_hook(&profile, &POWERSHELL_SPEC)
            .expect("current hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn current_powershell_hook_does_not_redirect_flush_output_handle() {
        assert!(POWERSHELL_HOOK.contains("sivtr flush"));
        assert!(!POWERSHELL_HOOK.contains("*>$null"));
    }

    #[test]
    fn replaces_existing_bash_block() {
        let profile = format!("before\n{BASH_HOOK}\nafter\n");
        let updated =
            update_existing_hook(&profile, &BASH_SPEC).expect("bash hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn bash_hook_sets_up_capture_file_and_debug_trap() {
        assert!(BASH_HOOK.contains("export SIVTR_CAPTURE_FILE="));
        assert!(BASH_HOOK.contains("trap '__sivtr_preexec' DEBUG"));
        assert!(BASH_HOOK.contains("tee \"$SIVTR_CAPTURE_FILE\""));
    }

    #[test]
    fn replaces_existing_zsh_block() {
        let profile = format!("before\n{ZSH_HOOK}\nafter\n");
        let updated =
            update_existing_hook(&profile, &ZSH_SPEC).expect("zsh hook should be detected");

        assert_eq!(updated, profile);
    }

    #[test]
    fn zsh_hook_sets_up_preexec_capture() {
        assert!(ZSH_HOOK.contains("export SIVTR_CAPTURE_FILE="));
        assert!(ZSH_HOOK.contains("preexec_functions=(_sivtr_preexec"));
        assert!(ZSH_HOOK.contains("typeset -ga preexec_functions precmd_functions"));
        assert!(ZSH_HOOK.contains("tee \"$SIVTR_CAPTURE_FILE\""));
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
        assert!(command.contains("cd \"$PROJECT_CWD\"; exec sivtr copy codex --pick"));
    }

    #[cfg(unix)]
    #[test]
    fn shell_single_quote_escapes_single_quotes() {
        assert_eq!(shell_single_quote("/tmp/it's"), "/tmp/it'\"'\"'s");
    }

    #[cfg(unix)]
    #[test]
    fn linux_shortcut_script_exports_project_cwd() {
        let script = render_linux_shortcut_script(Path::new("/tmp/project"), Some("xterm"));

        assert!(script.contains("export PROJECT_CWD='/tmp/project'"));
        assert!(script.contains("cd \"$PROJECT_CWD\"; exec sivtr copy codex --pick"));
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
        let script = render_macos_shortcut_script(Path::new("/tmp/project"));

        assert!(script.contains("export PROJECT_CWD='/tmp/project'"));
        assert!(script.contains("exec sivtr copy codex --pick"));
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

    #[cfg(windows)]
    #[test]
    fn hotkey_prompt_defaults_to_yes() {
        assert!(parse_yes_no_default_yes(""));
        assert!(parse_yes_no_default_yes("\n"));
        assert!(parse_yes_no_default_yes("y"));
        assert!(parse_yes_no_default_yes("YES"));
    }

    #[cfg(windows)]
    #[test]
    fn hotkey_prompt_accepts_no() {
        assert!(!parse_yes_no_default_yes("n"));
        assert!(!parse_yes_no_default_yes("No\n"));
    }
}
