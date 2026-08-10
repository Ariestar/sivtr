use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
pub struct McpCommand {
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Subcommand, Debug)]
pub enum McpAction {
    /// Run the read-only MCP server on stdio
    Serve(McpServeArgs),

    /// Install sivtr MCP into agent hosts (interactively picks hosts when -p is omitted)
    Install(McpInstallArgs),

    /// Remove sivtr MCP from agent hosts
    Uninstall(McpInstallArgs),

    /// Print MCP config snippet without writing files
    PrintConfig {
        /// Target agent command name (see `AgentProvider::command_names()`)
        target: String,
    },
}

#[derive(Args, Debug, Clone, Default)]
pub struct McpServeArgs {
    /// Exit the server process after this many seconds with no tool calls.
    /// The host respawns the server on the next tool use, so an idle server
    /// never lingers (each agent session otherwise keeps one alive until it
    /// exits). 0 = stay alive until the host closes stdin. Absent = the
    /// `[mcp] idle_exit_secs` config value (default 60; 0 disables).
    #[arg(long, value_name = "SECS")]
    pub idle_exit: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct McpInstallArgs {
    /// Provider host(s) to inject (comma-separated or repeated).
    /// Use registered command names, or `all`.
    /// Omit to pick hosts interactively; with -y, installs to detected hosts.
    #[arg(
        short = 'p',
        long = "provider",
        value_delimiter = ',',
        num_args = 1..
    )]
    pub providers: Vec<String>,

    /// Install location: global or local (project cwd)
    #[arg(short = 'l', long = "location", value_enum, default_value_t = McpLocation::Global)]
    pub location: McpLocation,

    /// Non-interactive: skip the host picker and install to detected hosts
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum McpLocation {
    #[default]
    Global,
    Local,
}
