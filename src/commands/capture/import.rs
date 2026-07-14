use anyhow::Result;
use sivtr_core::buffer::Buffer;
use sivtr_core::capture::scrollback;
use sivtr_core::claude_export::{import_claude_export, ClaudeExportImportOptions};
use sivtr_core::config::{OpenMode, SivtrConfig};
use sivtr_core::export::editor;
use sivtr_core::history::store::CaptureSource;
use sivtr_core::parse;

use super::browse;
use crate::app::App;
use crate::cli::{ImportAction, ImportCommand};
use crate::command_blocks;

pub fn execute(command: ImportCommand) -> Result<()> {
    match command.action {
        Some(ImportAction::ClaudeExport(args)) => execute_claude_export(args),
        None => execute_shell_session(),
    }
}

fn execute_claude_export(args: crate::cli::ClaudeExportImportArgs) -> Result<()> {
    let report = import_claude_export(&ClaudeExportImportOptions {
        source_dir: args.export_dir,
        cwd: args.cwd,
        destination: args.dest,
        dry_run: args.dry_run,
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let action = if report.already_imported {
        "already imported"
    } else if report.dry_run {
        "validated"
    } else {
        "imported"
    };
    println!(
        "sivtr: {action} {} conversations and {} design chats as {} Claude sessions",
        report.conversation_count, report.design_chat_count, report.generated_session_count
    );
    println!("  messages: {}", report.source_message_count);
    println!("  batch: {}", report.batch_id);
    println!("  destination: {}", report.destination);
    for warning in &report.warnings {
        eprintln!("sivtr: warning: {warning}");
    }
    Ok(())
}

/// Open the current session log.
fn execute_shell_session() -> Result<()> {
    match scrollback::read_session_log()? {
        Some(raw) => {
            if raw.trim().is_empty() {
                eprintln!("sivtr: session log is empty");
                return Ok(());
            }

            let config = SivtrConfig::load().unwrap_or_default();
            browse::record_history(&config, &raw, Some("sivtr import"), CaptureSource::Import);

            match config.general.open_mode {
                OpenMode::Editor => {
                    let ed = editor::resolve_editor_with_config(&config)?;
                    eprintln!("sivtr: opening session log in {ed}");
                    editor::open_in_editor(&raw)?;
                    Ok(())
                }
                OpenMode::Tui => {
                    let lines = parse::parse_lines(&raw);
                    let buffer = Buffer::new(lines);
                    let mut app = App::new(buffer);
                    app.config = config;
                    app.command_blocks =
                        command_blocks::load_from_session_log()?.unwrap_or_default();
                    browse::run_tui(&mut app, true)
                }
            }
        }
        None => {
            eprintln!("sivtr: no session log found");
            eprintln!("  hint: run `sivtr init <shell>` then restart your terminal");
            Ok(())
        }
    }
}
