use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
pub struct PublishCommand {
    #[command(subcommand)]
    pub action: Option<PublishAction>,
    /// Source ref or saved WorkSet name; default action is publish.
    #[arg(value_name = "SOURCE")]
    pub source: Option<String>,
    /// Optional public title for the default publish action
    #[arg(long)]
    pub title: Option<String>,
    /// Link lifetime for the default publish action
    #[arg(long, default_value = "7d")]
    pub expires: String,
    /// Confirm the default publish action without a prompt
    #[arg(long)]
    pub yes: bool,
    /// Allow non-automatic privacy warnings
    #[arg(long)]
    pub allow_warnings: bool,
}

#[derive(Subcommand, Debug)]
pub enum PublishAction {
    /// Build the final public snapshot locally without contacting the service
    Preview(PublishPreviewArgs),
    /// List local publication metadata and clickable active links in a TTY
    List(PublishListArgs),
    /// Print one complete browser link; choose one interactively when omitted
    Link(PublishIdArgs),
    /// Revoke one publication; choose one interactively when omitted
    Revoke(PublishRevokeArgs),
}

#[derive(Args, Debug, Clone)]
pub struct PublishPreviewArgs {
    /// Optional source ref or saved WorkSet name; omit it to open the TUI
    pub source: Option<String>,
    /// Optional public title
    #[arg(long)]
    pub title: Option<String>,
    /// Link lifetime: 2h, 1d, 3d, 7d, or 30d
    #[arg(long, default_value = "7d")]
    pub expires: String,
    /// Output format
    #[arg(long, value_enum, default_value_t = PublishFormat::Human)]
    pub format: PublishFormat,
}

#[derive(Args, Debug, Clone)]
pub struct PublishArgs {
    /// Source ref, saved WorkSet name, or @-prefixed WorkSet reference
    pub source: String,
    /// Optional public title
    #[arg(long)]
    pub title: Option<String>,
    /// Link lifetime: 2h, 1d, 3d, 7d, or 30d
    #[arg(long, default_value = "7d")]
    pub expires: String,
    /// Confirm without an interactive prompt
    #[arg(long)]
    pub yes: bool,
    /// Allow non-automatic privacy warnings in non-interactive mode
    #[arg(long)]
    pub allow_warnings: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PublishListArgs {
    /// Print machine-readable metadata
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PublishIdArgs {
    /// Publication id returned by `publish`
    pub publication_id: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PublishRevokeArgs {
    /// Publication id returned by `publish`
    pub publication_id: Option<String>,
    /// Confirm without an interactive prompt
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PublishFormat {
    Human,
    Json,
}
