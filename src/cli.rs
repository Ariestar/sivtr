use clap::{Args, Parser, Subcommand};

const COPY_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy` copies the last command block.
  The default content mode is input + output, with prompt preserved.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Selectors:
  sivtr copy           Last command block
  sivtr copy 3         Last 3 command blocks
  sivtr copy 2..5      From the 2nd-last to the 5th-last block
  sivtr copy --pick    Interactive multi-select picker

Content:
  --all                Input + output (default for `sivtr copy`)
  --in                 Input only
  --out                Output only
  --cmd                Bare command only, without prompt
  --prompt             Keep prompt in copied input

Filters:
  Filters run after the selected blocks are merged.
  --regex error
  --lines 10:20
  --lines 1,3,8:12

Examples:
  sivtr copy 3 --print
  sivtr copy --pick --regex panic
  sivtr copy 2..5 --out
  sivtr copy --cmd --print
";

const COPY_INPUT_AFTER_HELP: &str = "\
Defaults:
  `sivtr in` copies input from the last command block.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Selectors:
  sivtr in             Input from the last command block
  sivtr in 3           Input from the last 3 command blocks
  sivtr in 2..5        Input from the 2nd-last to the 5th-last block
  sivtr in --pick      Interactive multi-select picker

Filters:
  Filters run after the selected blocks are merged.
  If both are set, `--regex` runs before `--lines`.
  --regex error
  --lines 10:20
  --lines 1,3,8:12

Examples:
  sivtr in 2..4 --lines 1:5
  sivtr in --pick --regex cargo
  sivtr in 3 --print
  sivtr in --regex '^cargo '
";

const COPY_OUTPUT_AFTER_HELP: &str = "\
Defaults:
  `sivtr out` copies output from the last command block.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Selectors:
  sivtr out            Output from the last command block
  sivtr out 3          Output from the last 3 command blocks
  sivtr out 2..5       Output from the 2nd-last to the 5th-last block
  sivtr out --pick     Interactive multi-select picker

Filters:
  Filters run after the selected blocks are merged.
  If both are set, `--regex` runs before `--lines`.
  --regex error
  --lines 10:20
  --lines 1,3,8:12

Examples:
  sivtr out 3 --print
  sivtr out --pick --regex error
  sivtr out 2..5 --lines 1:20
  sivtr out --regex panic
";

/// sivtr 鈥?Terminal output workspace.
/// Capture, browse, search, select, and export terminal output.
#[derive(Parser, Debug)]
#[command(name = "sivtr", version, about, long_about = None)]
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

    /// Read from stdin pipe (e.g., `cmd | sivtr`)
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
    #[command(after_help = COPY_AFTER_HELP)]
    Copy(CopyArgs),

    /// Copy recent command input blocks to clipboard
    #[command(after_help = COPY_INPUT_AFTER_HELP)]
    In(CopyFilterArgs),

    /// Copy recent command output blocks to clipboard
    #[command(after_help = COPY_OUTPUT_AFTER_HELP)]
    Out(CopyFilterArgs),

    /// Clear the current session log
    Clear,

    /// Internal: flush console buffer to session log (called by shell hook)
    #[command(hide = true)]
    Flush,
}

#[derive(Args, Debug)]
pub struct CopyArgs {
    /// Which blocks to copy; `1` means the last block
    #[arg(value_name = "N|A..B")]
    pub selector: Option<String>,
    /// Open the interactive picker
    #[arg(long)]
    pub pick: bool,
    /// Copy input + output
    #[arg(long, conflicts_with_all = ["input", "output", "cmd"])]
    pub all: bool,
    /// Copy only input
    #[arg(long = "in", conflicts_with_all = ["all", "output", "cmd"])]
    pub input: bool,
    /// Copy only output
    #[arg(long = "out", conflicts_with_all = ["all", "input", "cmd"])]
    pub output: bool,
    /// Copy only the bare command, without prompt
    #[arg(long, conflicts_with_all = ["all", "input", "output", "prompt"])]
    pub cmd: bool,
    /// Keep the prompt in copied input
    #[arg(long, conflicts_with_all = ["output", "cmd"])]
    pub prompt: bool,
    /// Print the copied text after copying
    #[arg(long)]
    pub print: bool,
    /// Keep only lines matching this regex
    #[arg(long, value_name = "PATTERN")]
    pub regex: Option<String>,
    /// Keep only selected 1-based lines, for example `10:20` or `1,3,8:12`
    #[arg(long, value_name = "SPEC")]
    pub lines: Option<String>,
}

#[derive(Args, Debug)]
pub struct CopyFilterArgs {
    /// Which blocks to copy; `1` means the last block
    #[arg(value_name = "N|A..B")]
    pub selector: Option<String>,
    /// Open the interactive picker
    #[arg(long)]
    pub pick: bool,
    /// Print the copied text after copying
    #[arg(long)]
    pub print: bool,
    /// Keep only lines matching this regex
    #[arg(long, value_name = "PATTERN")]
    pub regex: Option<String>,
    /// Keep only selected 1-based lines, for example `10:20` or `1,3,8:12`
    #[arg(long, value_name = "SPEC")]
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
