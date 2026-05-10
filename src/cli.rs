use clap::{ArgGroup, Args, Parser, Subcommand};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sivtr_core::ai::AgentProvider;
use std::path::PathBuf;
use std::str::FromStr;

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

const COPY_CODEX_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy codex` copies the last completed user + assistant turn
  from the current Codex session.

Session Resolution:
  By default, sivtr reads the newest Codex rollout whose `cwd`
  matches the current working directory.
  `--session N` picks the Nth newest recorded Codex session.
  `--session ID` matches a session id or id prefix.

Selector Semantics:
  Selection is relative to the newest matching Codex item.
  `1` means the latest turn/message/tool output, `2` means the 2nd-latest.

Modes:
  sivtr copy codex       Copy the last user + assistant turn
  sivtr copy codex out   Copy the last assistant reply
  sivtr copy codex in    Copy the last user message
  sivtr copy codex tool  Copy the last tool output
  sivtr copy codex all   Copy the whole parsed session

Filters:
  Filters run after the selected text is assembled.
  If both are set, `--regex` runs before `--lines`.

Examples:
  sivtr copy codex
  sivtr copy codex --session 2
  sivtr copy codex --session 019df7fb
  sivtr copy codex 2
  sivtr copy codex 2..4
  sivtr copy codex out --session 2 --print
  sivtr copy codex --session 2 --pick
  sivtr copy codex out --pick
  sivtr copy codex tool --regex error
  sivtr copy codex all --lines 1:20
";

const COPY_CLAUDE_AFTER_HELP: &str = "\
Defaults:
  `sivtr copy claude` copies the last completed user + assistant turn
  from the current Claude Code session.

Session Resolution:
  By default, sivtr reads the newest Claude Code transcript whose `cwd`
  matches the current working directory.
  `--session N` picks the Nth newest recorded Claude session.
  `--session ID` matches a session id or id prefix.

Selector Semantics:
  Selection is relative to the newest matching Claude Code item.
  `1` means the latest turn/message/tool output, `2` means the 2nd-latest.

Modes:
  sivtr copy claude       Copy the last user + assistant turn
  sivtr copy claude out   Copy the last assistant reply
  sivtr copy claude in    Copy the last user message
  sivtr copy claude tool  Copy the last tool output
  sivtr copy claude all   Copy the whole parsed session

Examples:
  sivtr copy claude
  sivtr copy claude --session 2
  sivtr copy claude --session abc123
  sivtr copy claude out --print
  sivtr copy claude --session 2 --pick
  sivtr copy claude --pick
  sivtr copy claude all --lines 1:20
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

const HOTKEY_AFTER_HELP: &str = "\
Examples:
  sivtr hotkey start
  sivtr hotkey start --chord alt+y
  sivtr hotkey start --provider claude
  sivtr hotkey status
  sivtr hotkey stop

Behavior:
  The hotkey daemon registers one global shortcut and opens a new
  terminal window for picking from current AI sessions.
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
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
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

    /// Generate shell integration or Linux shortcut helpers
    Init {
        /// Integration target: powershell, bash, zsh, nushell, tmux, linux-shortcut
        shell: String,
    },

    /// Copy recent command blocks to clipboard
    #[command(visible_alias = "c", after_help = COPY_AFTER_HELP)]
    Copy(Box<CopyCommand>),

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

    /// Manage the global AI session picker hotkey
    #[command(after_help = HOTKEY_AFTER_HELP)]
    Hotkey(HotkeyCommand),

    /// Export local Codex session files into a shared read-only tree
    Codex(CodexCommand),

    /// Clear session logs
    Clear(ClearArgs),

    /// Internal: flush console buffer to session log (called by shell hook)
    #[command(hide = true)]
    Flush,

    /// Internal: run the Windows hotkey daemon loop
    #[command(hide = true)]
    HotkeyServe(HotkeyServeArgs),

    /// Internal: open the AI session picker from the Windows hotkey daemon
    #[command(hide = true)]
    HotkeyPickAgent(HotkeyPickAgentArgs),
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

    /// Copy content from the current Codex conversation session
    #[command(after_help = COPY_CODEX_AFTER_HELP)]
    Codex(AgentCopyCommand),

    /// Copy content from the current Claude Code conversation session
    #[command(after_help = COPY_CLAUDE_AFTER_HELP)]
    Claude(AgentCopyCommand),
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
pub struct AgentCopyArgs {
    #[command(flatten)]
    pub common: CopySimpleArgs,

    /// Which recorded session to read; `1` means the newest session, or pass an id / id prefix
    #[arg(long, value_name = "N|ID")]
    pub session: Option<String>,
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

#[derive(Parser, Debug)]
pub struct HotkeyCommand {
    #[command(subcommand)]
    pub action: Option<HotkeyAction>,
}

#[derive(Subcommand, Debug)]
pub enum HotkeyAction {
    /// Start the global hotkey daemon
    Start(HotkeyStartArgs),

    /// Stop the global hotkey daemon
    Stop,

    /// Show daemon status
    Status,
}

#[derive(Args, Debug, Clone)]
pub struct HotkeyStartArgs {
    /// Override the configured hotkey chord, for example `alt+y`
    #[arg(long, value_name = "CHORD")]
    pub chord: Option<String>,

    /// AI provider opened by the hotkey
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,
}

#[derive(Args, Debug, Clone)]
pub struct HotkeyServeArgs {
    /// Absolute working directory used when the picker terminal opens
    #[arg(long, value_name = "PATH")]
    pub cwd: String,

    /// Registered global hotkey chord, for example `alt+y`
    #[arg(long, value_name = "CHORD")]
    pub chord: String,

    /// AI provider opened by the hotkey
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,
}

#[derive(Args, Debug, Clone)]
pub struct HotkeyPickAgentArgs {
    /// Working directory used for launching the picker process
    #[arg(long, value_name = "PATH")]
    pub cwd: PathBuf,

    /// AI provider sessions to show
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,

    /// Restrict the picker to sessions whose cwd matches `--cwd`
    #[arg(long, default_value_t = false)]
    pub current_session: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotkeyProviderSelection(Option<AgentProvider>);

impl HotkeyProviderSelection {
    pub fn provider(provider: AgentProvider) -> Self {
        Self(Some(provider))
    }

    pub fn providers(self) -> Vec<AgentProvider> {
        match self.0 {
            Some(provider) => vec![provider],
            None => AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect(),
        }
    }

    pub fn as_str(self) -> &'static str {
        self.0.map(AgentProvider::command_name).unwrap_or("all")
    }

    pub fn label(self) -> &'static str {
        self.0
            .map(AgentProvider::name)
            .unwrap_or("all AI providers")
    }
}

impl FromStr for HotkeyProviderSelection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("all") {
            return Ok(Self::default());
        }

        AgentProvider::from_command_name(value)
            .map(Self::provider)
            .ok_or_else(|| format!("unknown AI provider `{value}`"))
    }
}

impl std::fmt::Display for HotkeyProviderSelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for HotkeyProviderSelection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HotkeyProviderSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Args, Debug)]
pub struct AgentCopyCommand {
    #[command(subcommand)]
    pub mode: Option<AgentCopyMode>,

    #[command(flatten)]
    pub args: AgentCopyArgs,
}

#[derive(Subcommand, Debug)]
pub enum AgentCopyMode {
    /// Copy the last user message
    In(AgentCopyArgs),

    /// Copy the last assistant reply
    Out(AgentCopyArgs),

    /// Copy the last tool output
    Tool(AgentCopyArgs),

    /// Copy the whole parsed session
    All(AgentCopyArgs),
}

#[derive(Parser, Debug)]
pub struct CodexCommand {
    #[command(subcommand)]
    pub action: CodexAction,
}

#[derive(Subcommand, Debug)]
pub enum CodexAction {
    /// Export local Codex rollout JSONL files into a target directory
    Export(CodexExportArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CodexExportArgs {
    /// Destination directory that will receive a sessions/ tree copy
    #[arg(long, value_name = "PATH")]
    pub dest: PathBuf,

    /// Keep only the newest N session files; `0` means export all
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub limit: usize,

    /// Continue mirroring local sessions into the destination tree
    #[arg(long, default_value_t = false)]
    pub watch: bool,

    /// Seconds between sync passes when `--watch` is enabled
    #[arg(long, value_name = "SECONDS", default_value_t = 1, requires = "watch")]
    pub interval: u64,

    /// Milliseconds between sync passes when `--watch` is enabled (overrides `--interval`)
    #[arg(long, value_name = "MILLISECONDS", requires = "watch")]
    pub interval_ms: Option<u64>,
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

    #[test]
    fn codex_copy_defaults_to_last_turn() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "codex"]).unwrap();

        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Codex(codex)) => {
                    assert!(codex.mode.is_none());
                    assert_eq!(codex.args.common.common.selector, None);
                }
                _ => panic!("expected copy codex mode"),
            },
            _ => panic!("expected copy command"),
        }
    }

    #[test]
    fn codex_copy_accepts_nested_mode() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "codex", "out", "--print"]).unwrap();

        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Codex(codex)) => match codex.mode {
                    Some(AgentCopyMode::Out(args)) => assert!(args.common.common.print),
                    _ => panic!("expected copy codex out mode"),
                },
                _ => panic!("expected copy codex mode"),
            },
            _ => panic!("expected copy command"),
        }
    }

    #[test]
    fn codex_copy_accepts_selector() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "codex", "2..4"]).unwrap();

        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Codex(codex)) => {
                    assert_eq!(codex.args.common.common.selector.as_deref(), Some("2..4"));
                }
                _ => panic!("expected copy codex mode"),
            },
            _ => panic!("expected copy command"),
        }
    }

    #[test]
    fn codex_copy_accepts_session_selector() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "codex", "--session", "2"]).unwrap();

        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Codex(codex)) => {
                    assert_eq!(codex.args.session.as_deref(), Some("2"));
                }
                _ => panic!("expected copy codex mode"),
            },
            _ => panic!("expected copy command"),
        }
    }

    #[test]
    fn claude_copy_accepts_nested_mode() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "claude", "out", "--print"]).unwrap();

        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Claude(claude)) => match claude.mode {
                    Some(AgentCopyMode::Out(args)) => assert!(args.common.common.print),
                    _ => panic!("expected copy claude out mode"),
                },
                _ => panic!("expected copy claude mode"),
            },
            _ => panic!("expected copy command"),
        }
    }

    #[test]
    fn claude_copy_accepts_session_selector() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "claude", "--session", "abc123"]).unwrap();

        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Claude(claude)) => {
                    assert_eq!(claude.args.session.as_deref(), Some("abc123"));
                }
                _ => panic!("expected copy claude mode"),
            },
            _ => panic!("expected copy command"),
        }
    }

    #[test]
    fn hotkey_start_accepts_chord_override() {
        let cli = Cli::try_parse_from(["sivtr", "hotkey", "start", "--chord", "alt+y"]).unwrap();

        match cli.command {
            Some(Commands::Hotkey(cmd)) => match cmd.action {
                Some(HotkeyAction::Start(args)) => {
                    assert_eq!(args.chord.as_deref(), Some("alt+y"));
                    assert_eq!(args.provider, HotkeyProviderSelection::default());
                }
                _ => panic!("expected hotkey start"),
            },
            _ => panic!("expected hotkey command"),
        }
    }

    #[test]
    fn hotkey_start_accepts_provider_override() {
        let cli =
            Cli::try_parse_from(["sivtr", "hotkey", "start", "--provider", "claude"]).unwrap();

        match cli.command {
            Some(Commands::Hotkey(cmd)) => match cmd.action {
                Some(HotkeyAction::Start(args)) => {
                    assert_eq!(
                        args.provider,
                        HotkeyProviderSelection::provider(AgentProvider::Claude)
                    );
                }
                _ => panic!("expected hotkey start"),
            },
            _ => panic!("expected hotkey command"),
        }
    }

    #[test]
    fn hotkey_pick_agent_defaults_to_all() {
        let cli = Cli::try_parse_from(["sivtr", "hotkey-pick-agent", "--cwd", "."]).unwrap();

        match cli.command {
            Some(Commands::HotkeyPickAgent(args)) => {
                assert_eq!(args.cwd, PathBuf::from("."));
                assert_eq!(args.provider, HotkeyProviderSelection::default());
                assert!(!args.current_session);
            }
            _ => panic!("expected hotkey-pick-agent command"),
        }
    }

    #[test]
    fn hotkey_pick_agent_accepts_current_session_flag() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "hotkey-pick-agent",
            "--cwd",
            ".",
            "--current-session",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::HotkeyPickAgent(args)) => {
                assert_eq!(args.cwd, PathBuf::from("."));
                assert!(args.current_session);
            }
            _ => panic!("expected hotkey-pick-agent command"),
        }
    }

    #[test]
    fn codex_export_accepts_destination_and_watch_flags() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "codex",
            "export",
            "--dest",
            "/tmp/shared-codex",
            "--limit",
            "5",
            "--watch",
            "--interval",
            "3",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Codex(cmd)) => match cmd.action {
                CodexAction::Export(args) => {
                    assert_eq!(args.dest, PathBuf::from("/tmp/shared-codex"));
                    assert_eq!(args.limit, 5);
                    assert!(args.watch);
                    assert_eq!(args.interval, 3);
                    assert_eq!(args.interval_ms, None);
                }
            },
            _ => panic!("expected codex export command"),
        }
    }

    #[test]
    fn codex_export_accepts_millisecond_interval() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "codex",
            "export",
            "--dest",
            "/tmp/shared-codex",
            "--watch",
            "--interval-ms",
            "250",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Codex(cmd)) => match cmd.action {
                CodexAction::Export(args) => {
                    assert_eq!(args.dest, PathBuf::from("/tmp/shared-codex"));
                    assert!(args.watch);
                    assert_eq!(args.interval, 1);
                    assert_eq!(args.interval_ms, Some(250));
                }
            },
            _ => panic!("expected codex export command"),
        }
    }

    #[test]
    fn hotkey_provider_rejects_unknown_provider() {
        let result = Cli::try_parse_from(["sivtr", "hotkey", "start", "--provider", "unknown"]);

        assert!(result.is_err());
    }

    #[test]
    fn run_accepts_hyphen_prefixed_child_args_without_separator() {
        let cli = Cli::try_parse_from(["sivtr", "run", "bash", "-lc", "printf ok"]).unwrap();

        match cli.command {
            Some(Commands::Run { command, args }) => {
                assert_eq!(command, "bash");
                assert_eq!(args, vec!["-lc".to_string(), "printf ok".to_string()]);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn init_accepts_tmux_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "tmux"]).unwrap();

        match cli.command {
            Some(Commands::Init { shell }) => assert_eq!(shell, "tmux"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_accepts_linux_shortcut_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "linux-shortcut"]).unwrap();

        match cli.command {
            Some(Commands::Init { shell }) => assert_eq!(shell, "linux-shortcut"),
            _ => panic!("expected init command"),
        }
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
