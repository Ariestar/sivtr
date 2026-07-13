use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
pub struct McpCommand {
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Subcommand, Debug)]
pub enum McpAction {
    /// Run the read-only MCP server on stdio
    Serve,

    /// Install sivtr MCP into agent hosts
    Install(McpInstallArgs),

    /// Remove sivtr MCP from agent hosts
    Uninstall(McpInstallArgs),

    /// Print MCP config snippet without writing files
    PrintConfig {
        /// Target agent id: claude, cursor, codex, opencode, pi, hermes
        target: String,
    },
}

#[derive(Args, Debug, Clone)]
pub struct McpInstallArgs {
    /// Target agent(s): claude, cursor, codex, opencode, pi, hermes, auto, all
    #[arg(short = 't', long = "target", default_value = "auto")]
    pub target: String,

    /// Install location: global or local (project cwd)
    #[arg(short = 'l', long = "location", value_enum, default_value_t = McpLocation::Global)]
    pub location: McpLocation,

    /// Non-interactive defaults
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum McpLocation {
    #[default]
    Global,
    Local,
}
