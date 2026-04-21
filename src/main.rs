mod app;
mod cli;
mod command_blocks;
mod commands;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands, CopyArgs, CopySimpleArgs, CopySubcommand};
use commands::copy::{CopyMode, CopyRequest};

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
        Some(Commands::Copy(args)) => match args.mode {
            Some(CopySubcommand::In(sub_args)) => {
                run_copy(&sub_args, CopyMode::InputOnly, true)?
            }
            Some(CopySubcommand::Out(sub_args)) => {
                run_copy_simple(&sub_args, CopyMode::OutputOnly, false)?
            }
            Some(CopySubcommand::Cmd(sub_args)) => {
                run_copy_simple(&sub_args, CopyMode::CommandOnly, false)?
            }
            None => run_copy(&args.args, CopyMode::Both, true)?,
        },
        Some(Commands::Ci(args)) => run_copy(&args, CopyMode::InputOnly, true)?,
        Some(Commands::Co(args)) => run_copy_simple(&args, CopyMode::OutputOnly, false)?,
        Some(Commands::Cc(args)) => run_copy_simple(&args, CopyMode::CommandOnly, false)?,
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
                // No pipe: open the current session log
                commands::import::execute()?;
            }
        }
    }

    Ok(())
}

fn run_copy(args: &CopyArgs, mode: CopyMode, include_prompt: bool) -> Result<()> {
    commands::copy::execute(CopyRequest {
        selector: args.common.selector.as_deref(),
        pick: args.common.pick,
        mode,
        include_prompt,
        prompt_override: args.prompt.as_deref(),
        print_full: args.common.print,
        regex: args.common.regex.as_deref(),
        lines: args.common.lines.as_deref(),
    })
}

fn run_copy_simple(args: &CopySimpleArgs, mode: CopyMode, include_prompt: bool) -> Result<()> {
    commands::copy::execute(CopyRequest {
        selector: args.common.selector.as_deref(),
        pick: args.common.pick,
        mode,
        include_prompt,
        prompt_override: None,
        print_full: args.common.print,
        regex: args.common.regex.as_deref(),
        lines: args.common.lines.as_deref(),
    })
}
