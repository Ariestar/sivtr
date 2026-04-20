mod cli;
mod app;
mod tui;
mod commands;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { command, args }) => {
            commands::run::execute(&command, &args)?;
        }
        Some(Commands::Pipe) => {
            commands::pipe::execute()?;
        }
        Some(Commands::Import) => {
            commands::import::execute()?;
        }
        Some(Commands::History(hist_cmd)) => {
            commands::history::execute(hist_cmd)?;
        }
        Some(Commands::Config(cfg_cmd)) => {
            commands::config::execute(cfg_cmd)?;
        }
        Some(Commands::Init { shell }) => {
            commands::init::execute(&shell)?;
        }
        Some(Commands::Clear) => {
            commands::clear::execute()?;
        }
        Some(Commands::Flush) => {
            commands::flush::execute()?;
        }
        None => {
            if atty::isnt(atty::Stream::Stdin) {
                // Piped input: read stdin
                commands::pipe::execute()?;
            } else {
                // No pipe: capture current terminal scrollback
                commands::import::execute()?;
            }
        }
    }

    Ok(())
}
