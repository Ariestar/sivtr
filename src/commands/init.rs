use anyhow::Result;

/// Generate shell integration hook code.
pub fn execute(shell: &str) -> Result<()> {
    match shell.to_lowercase().as_str() {
        "powershell" | "pwsh" => print_powershell_hook(),
        "bash" => print_bash_hook(),
        "zsh" => print_zsh_hook(),
        "fish" => print_fish_hook(),
        _ => {
            eprintln!(
                "sift: unsupported shell '{}'. Supported: powershell, bash, zsh, fish",
                shell
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

fn print_powershell_hook() {
    println!(
        r#"# sift shell integration for PowerShell
# Usage: sift init powershell | Invoke-Expression
# Permanent: Add the above line to $PROFILE
$env:SIFT_SESSION_LOG = Join-Path $env:APPDATA "sift\session_$PID.log"
$_sift_orig_prompt = $function:prompt
function prompt {{ try {{ sift flush *>$null }} catch {{}}; & $_sift_orig_prompt }}"#
    );
}

fn print_bash_hook() {
    println!(
        r#"# sift shell integration for Bash
# Usage: eval "$(sift init bash)"
# Permanent: Add the above line to ~/.bashrc
export HISTSIZE=32767
export SAVEHIST=32767"#
    );
}

fn print_zsh_hook() {
    println!(
        r#"# sift shell integration for Zsh
# Usage: eval "$(sift init zsh)"
# Permanent: Add the above line to ~/.zshrc
export HISTSIZE=32767
export SAVEHIST=32767"#
    );
}

fn print_fish_hook() {
    println!(
        r#"# sift shell integration for Fish
# Usage: sift init fish | source
# Permanent: Add the above line to ~/.config/fish/config.fish
set -g fish_history_size 32767"#
    );
}
