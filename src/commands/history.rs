use anyhow::Result;
use sift_core::history::HistoryStore;
use crate::cli::{HistoryCommand, HistoryAction};

/// Execute history subcommands.
pub fn execute(cmd: HistoryCommand) -> Result<()> {
    let store = HistoryStore::open_default()?;

    match cmd.action {
        Some(HistoryAction::List { limit }) => {
            let entries = store.list_recent(limit)?;
            if entries.is_empty() {
                println!("No history entries found.");
                return Ok(());
            }
            for entry in &entries {
                let cmd_str = entry.command.as_deref().unwrap_or("-");
                let preview: String = entry.content.lines().next().unwrap_or("").chars().take(60).collect();
                println!(
                    "{:>5}  {}  [{}]  {}",
                    entry.id, &entry.timestamp[..19], cmd_str, preview
                );
            }
        }
        Some(HistoryAction::Search { keyword, limit }) => {
            let entries = store.search(&keyword, limit)?;
            if entries.is_empty() {
                println!("No matches found for '{}'.", keyword);
                return Ok(());
            }
            for entry in &entries {
                let cmd_str = entry.command.as_deref().unwrap_or("-");
                let preview: String = entry.content.lines().next().unwrap_or("").chars().take(60).collect();
                println!(
                    "{:>5}  {}  [{}]  {}",
                    entry.id, &entry.timestamp[..19], cmd_str, preview
                );
            }
        }
        Some(HistoryAction::Show { id }) => {
            match store.get_by_id(id)? {
                Some(entry) => {
                    println!("--- History #{} ---", entry.id);
                    println!("Timestamp: {}", entry.timestamp);
                    println!("Command:   {}", entry.command.as_deref().unwrap_or("-"));
                    println!("Source:    {}", entry.source);
                    println!("Host:      {}", entry.hostname);
                    println!("---");
                    println!("{}", entry.content);
                }
                None => {
                    println!("History entry #{} not found.", id);
                }
            }
        }
        None => {
            // Default: list recent
            let entries = store.list_recent(20)?;
            if entries.is_empty() {
                println!("No history entries found.");
                return Ok(());
            }
            for entry in &entries {
                let cmd_str = entry.command.as_deref().unwrap_or("-");
                let preview: String = entry.content.lines().next().unwrap_or("").chars().take(60).collect();
                println!(
                    "{:>5}  {}  [{}]  {}",
                    entry.id, &entry.timestamp[..19], cmd_str, preview
                );
            }
        }
    }

    Ok(())
}
