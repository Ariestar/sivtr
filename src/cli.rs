use clap::{Args, Parser, Subcommand};

/// sift — Terminal output workspace.
/// Capture, browse, search, select, and export terminal output.
#[derive(Parser, Debug)]
#[command(name = "sift", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Wrap a command execution and capture its output
    Run {
        /// The command to run
        command: String,
        /// Arguments to pass to the command
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Read from stdin pipe (e.g., `cmd | sift`)
    Pipe,

    /// Import scrollback from the current terminal multiplexer
    Import,

    /// Manage output history
    History(HistoryCommand),

    /// Manage configuration
    Config(ConfigCommand),

    /// Generate shell integration hook
    Init {
        /// Shell type: powershell, bash, zsh, nushell
        shell: String,
    },

    /// Copy recent command blocks to clipboard
    Copy(CopyArgs),

    /// Copy recent command input blocks to clipboard
    In(CopyFilterArgs),

    /// Copy recent command output blocks to clipboard
    Out(CopyFilterArgs),

    /// Clear the current session log
    Clear,

    /// Internal: flush console buffer to session log (called by shell hook)
    #[command(hide = true)]
    Flush,
}

#[derive(Args, Debug)]
pub struct CopyArgs {
    /// Command selector: N for recent N commands, or A..B for the Nth-last range
    pub selector: Option<String>,
    /// Interactively select command blocks to copy
    #[arg(long)]
    pub pick: bool,
    /// Copy input + output explicitly (default behavior)
    #[arg(long, conflicts_with_all = ["input", "output", "cmd"])]
    pub all: bool,
    /// Copy only command input/prompt lines
    #[arg(long = "in", conflicts_with_all = ["all", "output", "cmd"])]
    pub input: bool,
    /// Copy only command output lines
    #[arg(long = "out", conflicts_with_all = ["all", "input", "cmd"])]
    pub output: bool,
    /// Copy only the bare command text, without prompt/context
    #[arg(long, conflicts_with_all = ["all", "input", "output", "prompt"])]
    pub cmd: bool,
    /// Preserve prompt/context explicitly (already default for `sift copy`)
    #[arg(long, conflicts_with_all = ["output", "cmd"])]
    pub prompt: bool,
    /// Print the full copied text to the terminal
    #[arg(long)]
    pub print: bool,
    /// Filter copied text by regex, keeping matching lines
    #[arg(long)]
    pub regex: Option<String>,
    /// Select specific 1-based line numbers/ranges, e.g. 1,3-5,8
    #[arg(long)]
    pub lines: Option<String>,
}

#[derive(Args, Debug)]
pub struct CopyFilterArgs {
    /// Command selector: N for recent N commands, or A..B for the Nth-last range
    pub selector: Option<String>,
    /// Interactively select command blocks to copy
    #[arg(long)]
    pub pick: bool,
    /// Print the full copied text to the terminal
    #[arg(long)]
    pub print: bool,
    /// Filter copied text by regex, keeping matching lines
    #[arg(long)]
    pub regex: Option<String>,
    /// Select specific 1-based line numbers/ranges, e.g. 1,3-5,8
    #[arg(long)]
    pub lines: Option<String>,
}

#[derive(Parser, Debug)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current config file path and contents
    Show,
    /// Create default config file if it doesn't exist
    Init,
    /// Open config file in editor
    Edit,
}

#[derive(Parser, Debug)]
pub struct HistoryCommand {
    #[command(subcommand)]
    pub action: Option<HistoryAction>,
}

#[derive(Subcommand, Debug)]
pub enum HistoryAction {
    /// Search history by keyword
    Search {
        /// Search keyword
        keyword: String,
        /// Maximum number of results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Show a specific history entry
    Show {
        /// History entry ID
        id: i64,
    },
    /// List recent history entries
    List {
        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
}
