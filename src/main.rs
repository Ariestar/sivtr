mod app;
mod cli;
mod command_blocks;
mod commands;
mod mcp;
mod output;
mod remote;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{
    AgentCopyArgs, AgentCopyCommand, AgentCopyMode, Cli, Commands, CopyArgs, CopyRefArgs,
    CopySimpleArgs, CopySubcommand, DiffArgs, HotkeyPickAgentArgs, HotkeyServeArgs,
};
use command_blocks::CommandBlockTextMode;
use commands::capture::copy::{AgentCopyRequest, AgentPickerRequest, CopyMode, CopyRequest};
use commands::capture::diff::DiffRequest;
use sivtr_core::ai::{AgentProvider, AgentSelection};

use std::process::ExitCode;

fn main() -> ExitCode {
    output::configure_utf8_console();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if commands::capture::copy::is_pick_cancelled(&error) => ExitCode::SUCCESS,
        Err(error) => {
            output::error(format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    output::set_color_choice(cli.color.into());

    match cli.command {
        Some(Commands::Run { command, args }) => {
            commands::capture::run::execute(&command, &args)?;
        }
        Some(Commands::Pipe) => {
            commands::capture::pipe::execute()?;
        }
        Some(Commands::Import(command)) => {
            commands::capture::import::execute(command)?;
        }
        Some(Commands::History(hist_cmd)) => {
            commands::system::history::execute(hist_cmd)?;
        }
        Some(Commands::Search(args)) => {
            commands::memory::search::execute(&args)?;
        }
        Some(Commands::Filter(args)) => {
            commands::memory::filter::execute(&args)?;
        }
        Some(Commands::Var(command)) => {
            commands::memory::var::execute(&command)?;
        }
        Some(Commands::Nav(args)) => {
            commands::memory::nav::execute(&args)?;
        }
        Some(Commands::Zoom(args)) => {
            commands::memory::zoom::execute(&args)?;
        }
        Some(Commands::Work(cmd)) => {
            commands::memory::work::execute(&cmd)?;
        }
        Some(Commands::Workspace(cmd)) => {
            commands::remote::workspace::execute(cmd)?;
        }
        Some(Commands::Show(args)) => {
            commands::memory::show::execute(&args)?;
        }
        Some(Commands::Mcp(cmd)) => {
            commands::system::mcp::execute(cmd)?;
        }
        Some(Commands::Serve(args)) => {
            commands::remote::serve::execute(&args)?;
        }
        Some(Commands::Share(cmd)) => {
            commands::remote::share::execute(cmd)?;
        }
        Some(Commands::Peer(cmd)) => {
            commands::remote::peer::execute(cmd)?;
        }
        Some(Commands::Remote(cmd)) => {
            commands::remote::execute(cmd)?;
        }
        Some(Commands::Hotkey(cmd)) => {
            commands::system::hotkey::execute(cmd)?;
        }
        Some(Commands::Codex(cmd)) => {
            commands::system::codex::execute(cmd)?;
        }
        Some(Commands::Config(cfg_cmd)) => {
            commands::system::config::execute(cfg_cmd)?;
        }
        Some(Commands::Doctor(args)) => {
            commands::system::doctor::execute(args)?;
        }
        Some(Commands::Setup) => {
            commands::system::setup::execute()?;
        }
        Some(Commands::Init { target }) => {
            commands::capture::init::execute(&target)?;
        }
        Some(Commands::Copy(args)) => match args.mode {
            Some(CopySubcommand::In(sub_args)) => run_copy(&sub_args, CopyMode::InputOnly, true)?,
            Some(CopySubcommand::Out(sub_args)) => {
                run_copy_simple(&sub_args, CopyMode::OutputOnly, false)?
            }
            Some(CopySubcommand::Cmd(sub_args)) => {
                run_copy_simple(&sub_args, CopyMode::CommandOnly, false)?
            }
            Some(CopySubcommand::Ref(sub_args)) => run_copy_ref(&sub_args)?,
            Some(CopySubcommand::Claude(sub_args)) => {
                run_agent_copy(AgentProvider::Claude, sub_args)?
            }
            Some(CopySubcommand::Codex(sub_args)) => {
                run_agent_copy(AgentProvider::Codex, sub_args)?
            }
            Some(CopySubcommand::OpenCode(sub_args)) => {
                run_agent_copy(AgentProvider::OpenCode, sub_args)?
            }
            Some(CopySubcommand::Hermes(sub_args)) => {
                run_agent_copy(AgentProvider::Hermes, sub_args)?
            }
            Some(CopySubcommand::Pi(sub_args)) => run_agent_copy(AgentProvider::Pi, sub_args)?,
            None => run_copy(&args.args, CopyMode::Both, true)?,
        },
        Some(Commands::Ci(args)) => run_copy(&args, CopyMode::InputOnly, true)?,
        Some(Commands::Co(args)) => run_copy_simple(&args, CopyMode::OutputOnly, false)?,
        Some(Commands::Cc(args)) => run_copy_simple(&args, CopyMode::CommandOnly, false)?,
        Some(Commands::Diff(args)) => {
            run_diff(&args)?;
        }
        Some(Commands::Version(args)) => {
            commands::system::version::execute(args.verbose)?;
        }
        Some(Commands::Clear(args)) => {
            commands::capture::clear::execute(args.all)?;
        }
        Some(Commands::Flush) => {
            commands::capture::flush::execute()?;
        }
        Some(Commands::HotkeyServe(args)) => {
            run_hotkey_serve(&args)?;
        }
        Some(Commands::HotkeyPickAgent(args)) => {
            run_hotkey_pick_agent(&args)?;
        }
        Some(Commands::ServeDaemon) => {
            remote::daemon::run()?;
        }
        None => {
            if atty::isnt(atty::Stream::Stdin) {
                // Piped input: read stdin
                commands::capture::pipe::execute()?;
            } else {
                run_workspace()?;
            }
        }
    }

    Ok(())
}

fn run_workspace() -> Result<()> {
    let providers = AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect::<Vec<_>>();
    commands::capture::copy::execute_agent_picker(AgentPickerRequest {
        providers: &providers,
        pick_current_session: true,
        selection_mode: AgentSelection::LastTurn,
        print_full: false,
        regex: None,
        lines: None,
    })
}

fn run_copy(args: &CopyArgs, mode: CopyMode, include_prompt: bool) -> Result<()> {
    commands::capture::copy::execute(CopyRequest {
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
    commands::capture::copy::execute(CopyRequest {
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

fn run_copy_ref(args: &CopyRefArgs) -> Result<()> {
    commands::capture::copy::execute_ref(
        &args.reference,
        args.cwd.as_deref(),
        args.print,
        args.regex.as_deref(),
        args.lines.as_deref(),
    )
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
    commands::capture::copy::execute_agent(AgentCopyRequest {
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

    commands::capture::diff::execute(DiffRequest {
        left_selector: &args.left,
        right_selector: &args.right,
        mode,
        side_by_side: args.side_by_side,
    })
}

fn run_hotkey_serve(args: &HotkeyServeArgs) -> Result<()> {
    commands::system::hotkey::serve(args)
}

fn run_hotkey_pick_agent(args: &HotkeyPickAgentArgs) -> Result<()> {
    commands::system::hotkey::pick_agent(args)
}
