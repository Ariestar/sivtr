mod cli;
mod app;
mod tui;
mod commands;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};
use commands::copy::CopyMode;

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
        Some(Commands::Copy(args)) => {
            let mode = if args.cmd {
                CopyMode::CommandOnly
            } else if args.input {
                CopyMode::InputOnly
            } else if args.output {
                CopyMode::OutputOnly
            } else {
                CopyMode::Both
            };
            commands::copy::execute(
                args.selector.as_deref(),
                args.pick,
                mode,
                args.prompt || !args.cmd,
                args.print,
                args.regex.as_deref(),
                args.lines.as_deref(),
            )?;
        }
        Some(Commands::In(args)) => {
            commands::copy::execute(
                args.selector.as_deref(),
                args.pick,
                CopyMode::InputOnly,
                false,
                args.print,
                args.regex.as_deref(),
                args.lines.as_deref(),
            )?;
        }
        Some(Commands::Out(args)) => {
            commands::copy::execute(
                args.selector.as_deref(),
                args.pick,
                CopyMode::OutputOnly,
                false,
                args.print,
                args.regex.as_deref(),
                args.lines.as_deref(),
            )?;
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
