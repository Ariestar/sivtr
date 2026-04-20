use clap::{Parser, Subcommand};

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

    /// Copy the output of the Nth-last command to clipboard
    Copy {
        /// How many commands back (default: 1 = last command)
        #[arg(default_value = "1")]
        n: usize,
    },

    /// Clear the current session log
    Clear,

    /// Internal: flush console buffer to session log (called by shell hook)
    #[command(hide = true)]
    Flush,
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
