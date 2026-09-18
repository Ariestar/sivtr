use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct PtyProxyCommand {
    #[command(subcommand)]
    pub action: PtyProxyAction,
}

#[derive(Subcommand, Debug)]
pub enum PtyProxyAction {
    /// Internal: run a shell inside the capture pty (called by the rc block)
    #[command(hide = true)]
    Run {
        /// Shell program to run inside the pty
        command: String,
        /// Arguments to pass to the shell
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Internal: hand one finished command to the proxy (called by the prompt hook)
    #[command(hide = true)]
    Report {
        /// Shell history id of the finished command (used to skip prompt repeats)
        #[arg(long, default_value = "")]
        command_id: String,
        /// The command line that ran
        #[arg(long, default_value = "")]
        command: String,
        /// The prompt rendered for that command
        #[arg(long, default_value = "")]
        prompt: String,
        /// Working directory the command ran in
        #[arg(long, default_value = "")]
        cwd: String,
        /// Exit status of the command
        #[arg(long, default_value_t = 0)]
        exit: i32,
        /// The block opens with the echoed input (PowerShell, whose block can
        /// only start at the end of its prompt)
        #[arg(long)]
        echoed_input: bool,
    },

    /// Turn terminal capture on and install the shell integration
    Enable {
        /// Target shell: bash, zsh, nushell, powershell, or all (default)
        #[arg(default_value = "all")]
        shell: String,
    },

    /// Turn terminal capture off
    Disable,
}
