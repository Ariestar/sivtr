use clap::{ArgGroup, Args, Parser, Subcommand};

const COPY_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy` copies the last command block.
  The default content mode is input + output, with prompt preserved.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Prompt Output:
  `--prompt TEXT` rewrites the copied input prompt.
  Example: `sivtr copy --prompt ':'` produces `: cargo test`.

Modes:
  sivtr copy           Copy input + output
  sivtr copy in        Copy input only
  sivtr copy out       Copy output only
  sivtr copy cmd       Copy the bare command only

Aliases:
  sivtr c              Same as `sivtr copy`
  sivtr ci             Same as `sivtr copy in`
  sivtr co             Same as `sivtr copy out`
  sivtr cc             Same as `sivtr copy cmd`

Filters:
  Filters run after the selected blocks are merged.
  If both are set, `--regex` runs before `--lines`.
  --regex error
  --lines 10:20
  --lines 1,3,8:12

Examples:
  sivtr copy
  sivtr copy 3 --print
  sivtr copy --prompt \":\"
  sivtr copy in 2..4
  sivtr copy out --pick --regex panic
  sivtr copy cmd --pick
";

const COPY_INPUT_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy in` copies input from the last command block.
  Prompt is preserved by default.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Examples:
  sivtr copy in
  sivtr copy in 3 --print
  sivtr copy in --prompt \":\"
  sivtr copy in 2..5 --lines 1:5
  sivtr copy in --pick --regex cargo
";

const COPY_OUTPUT_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy out` copies output from the last command block.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Examples:
  sivtr copy out
  sivtr copy out 3 --print
  sivtr copy out 2..5 --lines 1:20
  sivtr copy out --pick --regex error
";

const COPY_COMMAND_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy cmd` copies the bare command from the last command block.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.

Examples:
  sivtr copy cmd
  sivtr copy cmd 3 --print
  sivtr copy cmd --pick
  sivtr copy cmd 2..5
";

const DIFF_AFTER_HELP: &str = "\
Defaults:
  `sivtr diff <left> <right>` compares two command blocks from the current session.
  The default content mode is `--output`.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.
  Each selector must resolve to exactly one block.

Modes:
  --output         Compare command output (default)
  --block          Compare input + output
  --input          Compare input with prompt
  --cmd            Compare bare command text

View:
  Unified diff is the default output.
  --side-by-side   Show a two-column text view

Examples:
  sivtr diff 1 2
  sivtr diff 3 1 --block
  sivtr diff 2 1 --side-by-side
";

/// sivtr - Terminal output workspace.
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

    /// Open the current session log
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
    #[command(visible_alias = "c", after_help = COPY_AFTER_HELP)]
    Copy(CopyCommand),

    /// Alias for `copy in`
    #[command(name = "ci", hide = true)]
    Ci(CopyArgs),

    /// Alias for `copy out`
    #[command(name = "co", hide = true)]
    Co(CopySimpleArgs),

    /// Alias for `copy cmd`
    #[command(name = "cc", hide = true)]
    Cc(CopySimpleArgs),

    /// Compare two recent command blocks in the current session
    #[command(after_help = DIFF_AFTER_HELP)]
    Diff(DiffArgs),

    /// Clear session logs
    Clear(ClearArgs),

    /// Internal: flush console buffer to session log (called by shell hook)
    #[command(hide = true)]
    Flush,
}

#[derive(Args, Debug)]
pub struct CopyCommand {
    #[command(subcommand)]
    pub mode: Option<CopySubcommand>,

    #[command(flatten)]
    pub args: CopyArgs,
}

#[derive(Subcommand, Debug)]
pub enum CopySubcommand {
    /// Copy recent command input blocks to clipboard
    #[command(after_help = COPY_INPUT_AFTER_HELP)]
    In(CopyArgs),

    /// Copy recent command output blocks to clipboard
    #[command(after_help = COPY_OUTPUT_AFTER_HELP)]
    Out(CopySimpleArgs),

    /// Copy only the bare command text
    #[command(after_help = COPY_COMMAND_AFTER_HELP)]
    Cmd(CopySimpleArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CopyCommonArgs {
    /// Which blocks to copy; `1` means the last block
    #[arg(value_name = "N|A..B")]
    pub selector: Option<String>,

    /// Copy the ANSI-decorated version when available
    #[arg(long)]
    pub ansi: bool,

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

#[derive(Args, Debug, Clone)]
pub struct CopyArgs {
    #[command(flatten)]
    pub common: CopyCommonArgs,

    /// Prompt text used in copied input instead of the original shell prompt
    #[arg(long = "prompt", value_name = "TEXT")]
    pub prompt: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct CopySimpleArgs {
    #[command(flatten)]
    pub common: CopyCommonArgs,
}

#[derive(Args, Debug, Clone)]
#[command(group(
    ArgGroup::new("diff_content_mode")
        .args(["output", "block", "input", "cmd"])
        .multiple(false)
))]
pub struct DiffArgs {
    /// Left selector, for example `1`
    #[arg(value_name = "LEFT")]
    pub left: String,

    /// Right selector, for example `2`
    #[arg(value_name = "RIGHT")]
    pub right: String,

    /// Compare output text (default)
    #[arg(long)]
    pub output: bool,

    /// Compare input + output
    #[arg(long)]
    pub block: bool,

    /// Compare input with prompt
    #[arg(long)]
    pub input: bool,

    /// Compare bare command text
    #[arg(long)]
    pub cmd: bool,

    /// Show side-by-side text output instead of unified diff
    #[arg(long = "side-by-side")]
    pub side_by_side: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ClearArgs {
    /// Clear all recorded session logs and state files
    #[arg(short = 'a', long = "all")]
    pub all: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_input_accepts_prompt_override() {
        let cli = Cli::try_parse_from(["sivtr", "ci", "--prompt", ":"]).unwrap();

        match cli.command {
            Some(Commands::Ci(args)) => assert_eq!(args.prompt.as_deref(), Some(":")),
            _ => panic!("expected ci command"),
        }
    }

    #[test]
    fn copy_out_does_not_accept_prompt_override() {
        let result = Cli::try_parse_from(["sivtr", "co", "--prompt", ":"]);

        assert!(result.is_err());
    }

    #[test]
    fn copy_cmd_does_not_accept_prompt_argument() {
        let result = Cli::try_parse_from(["sivtr", "cc", "--prompt", ":"]);

        assert!(result.is_err());
    }

    #[test]
    fn copy_aliases_accept_ansi_flag() {
        let cli = Cli::try_parse_from(["sivtr", "co", "--ansi"]).unwrap();

        match cli.command {
            Some(Commands::Co(args)) => assert!(args.common.ansi),
            _ => panic!("expected co command"),
        }
    }

    #[test]
    fn clear_accepts_all_flag() {
        let cli = Cli::try_parse_from(["sivtr", "clear", "--all"]).unwrap();

        match cli.command {
            Some(Commands::Clear(args)) => assert!(args.all),
            _ => panic!("expected clear command"),
        }
    }

    #[test]
    fn diff_parses_two_selectors() {
        let cli = Cli::try_parse_from(["sivtr", "diff", "1", "2"]).unwrap();

        match cli.command {
            Some(Commands::Diff(args)) => {
                assert_eq!(args.left, "1");
                assert_eq!(args.right, "2");
                assert!(!args.side_by_side);
                assert!(!args.output);
                assert!(!args.block);
                assert!(!args.input);
                assert!(!args.cmd);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_parses_block_mode_and_side_by_side() {
        let cli =
            Cli::try_parse_from(["sivtr", "diff", "3", "1", "--block", "--side-by-side"]).unwrap();

        match cli.command {
            Some(Commands::Diff(args)) => {
                assert_eq!(args.left, "3");
                assert_eq!(args.right, "1");
                assert!(args.block);
                assert!(args.side_by_side);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_rejects_multiple_content_modes() {
        let result = Cli::try_parse_from(["sivtr", "diff", "1", "2", "--output", "--cmd"]);
        assert!(result.is_err());
    }
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
