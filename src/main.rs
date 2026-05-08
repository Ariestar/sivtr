mod app;
mod cli;
mod command_blocks;
mod commands;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{
    AgentCopyArgs, AgentCopyCommand, AgentCopyMode, Cli, Commands, CopyArgs, CopySimpleArgs,
    CopySubcommand, DiffArgs, HotkeyPickAgentArgs, HotkeyServeArgs,
};
use command_blocks::CommandBlockTextMode;
use commands::copy::{AgentCopyRequest, CopyMode, CopyRequest};
use commands::diff::DiffRequest;
use sivtr_core::ai::{AgentProvider, AgentSelection};

fn main() -> Result<()> {
    match run() {
        Ok(()) => Ok(()),
        Err(error) if commands::copy::is_pick_cancelled(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn run() -> Result<()> {
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
        Some(Commands::Hotkey(cmd)) => {
            commands::hotkey::execute(cmd)?;
        }
        Some(Commands::Config(cfg_cmd)) => {
            commands::config::execute(cfg_cmd)?;
        }
        Some(Commands::Init { shell }) => {
            commands::init::execute(&shell)?;
        }
        Some(Commands::Copy(args)) => match args.mode {
            Some(CopySubcommand::In(sub_args)) => run_copy(&sub_args, CopyMode::InputOnly, true)?,
            Some(CopySubcommand::Out(sub_args)) => {
                run_copy_simple(&sub_args, CopyMode::OutputOnly, false)?
            }
            Some(CopySubcommand::Cmd(sub_args)) => {
                run_copy_simple(&sub_args, CopyMode::CommandOnly, false)?
            }
            Some(CopySubcommand::Claude(sub_args)) => {
                run_agent_copy(AgentProvider::Claude, sub_args)?
            }
            Some(CopySubcommand::Codex(sub_args)) => {
                run_agent_copy(AgentProvider::Codex, sub_args)?
            }
            None => run_copy(&args.args, CopyMode::Both, true)?,
        },
        Some(Commands::Ci(args)) => run_copy(&args, CopyMode::InputOnly, true)?,
        Some(Commands::Co(args)) => run_copy_simple(&args, CopyMode::OutputOnly, false)?,
        Some(Commands::Cc(args)) => run_copy_simple(&args, CopyMode::CommandOnly, false)?,
        Some(Commands::Diff(args)) => {
            run_diff(&args)?;
        }
        Some(Commands::Clear(args)) => {
            commands::clear::execute(args.all)?;
        }
        Some(Commands::Flush) => {
            commands::flush::execute()?;
        }
        Some(Commands::HotkeyServe(args)) => {
            run_hotkey_serve(&args)?;
        }
        Some(Commands::HotkeyPickAgent(args)) => {
            run_hotkey_pick_agent(&args)?;
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
        ansi: args.common.ansi,
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
        ansi: args.common.ansi,
        regex: args.common.regex.as_deref(),
        lines: args.common.lines.as_deref(),
    })
}

fn run_agent_copy(provider: AgentProvider, cmd: AgentCopyCommand) -> Result<()> {
    match cmd.mode {
        Some(AgentCopyMode::In(args)) => {
            run_agent_copy_args(provider, &args, AgentSelection::LastUser)
        }
        Some(AgentCopyMode::Out(args)) => {
            run_agent_copy_args(provider, &args, AgentSelection::LastAssistant)
        }
        Some(AgentCopyMode::Tool(args)) => {
            run_agent_copy_args(provider, &args, AgentSelection::LastTool)
        }
        Some(AgentCopyMode::All(args)) => run_agent_copy_args(provider, &args, AgentSelection::All),
        None => run_agent_copy_args(provider, &cmd.args, AgentSelection::LastTurn),
    }
}

fn run_agent_copy_args(
    provider: AgentProvider,
    args: &AgentCopyArgs,
    selection_mode: AgentSelection,
) -> Result<()> {
    commands::copy::execute_agent(AgentCopyRequest {
        provider,
        selector: args.common.common.selector.as_deref(),
        session_selector: args.session.as_deref(),
        pick: args.common.common.pick,
        pick_current_session: false,
        selection_mode,
        print_full: args.common.common.print,
        regex: args.common.common.regex.as_deref(),
        lines: args.common.common.lines.as_deref(),
    })
}

fn run_diff(args: &DiffArgs) -> Result<()> {
    let mode = if args.block {
        CommandBlockTextMode::Block
    } else if args.input {
        CommandBlockTextMode::Input
    } else if args.cmd {
        CommandBlockTextMode::Command
    } else {
        CommandBlockTextMode::Output
    };

    commands::diff::execute(DiffRequest {
        left_selector: &args.left,
        right_selector: &args.right,
        mode,
        side_by_side: args.side_by_side,
    })
}

fn run_hotkey_serve(args: &HotkeyServeArgs) -> Result<()> {
    commands::hotkey::serve(args)
}

fn run_hotkey_pick_agent(args: &HotkeyPickAgentArgs) -> Result<()> {
    commands::hotkey::pick_agent(args)
}
