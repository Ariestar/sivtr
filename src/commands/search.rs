use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::{SearchArgs, SearchScopeArg};
use crate::commands::copy::current_workspace_sessions;
use crate::commands::workspace_json::{
    line_ref, workspace_item, workspace_source, WorkspaceJsonItem,
};
use crate::tui::workspace::WorkspaceSession;
use crate::tui::workspace_search::{
    WorkspaceSearchIndex, WorkspaceSearchMatch, WorkspaceSearchScope,
};
use sivtr_core::ai::AgentSelection;

#[derive(Serialize)]
struct SearchJsonOutput {
    query: String,
    scope: String,
    cwd: String,
    match_count: usize,
    results: Vec<WorkspaceJsonItem>,
}

struct SearchResult<'a> {
    session: &'a WorkspaceSession,
    dialogue_title: &'a str,
    dialogue_index: usize,
    line_index: usize,
    timestamp: Option<&'a str>,
    content: String,
}

pub fn execute(args: &SearchArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let providers = args.provider.providers();
    let sessions = current_workspace_sessions(&providers, &cwd, AgentSelection::LastTurn)?;
    let scope = search_scope(args.scope);
    let output =
        WorkspaceSearchIndex::new(&sessions).search_with_scope(&sessions, scope, &args.query);
    let results = collect_results(&output.sessions, &output.matches, args.limit);

    if args.json {
        let json = SearchJsonOutput {
            query: args.query.clone(),
            scope: search_scope_name(scope).to_string(),
            cwd: cwd.display().to_string(),
            match_count: output.matches.len(),
            results: results
                .iter()
                .map(|result| {
                    workspace_item(
                        result.session,
                        line_ref(result.session, result.dialogue_index, result.line_index),
                        Some(result.dialogue_title.to_string()),
                        result.timestamp.map(str::to_string),
                        result.content.clone(),
                    )
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    println!(
        "sivtr search: {} match(es) for {:?} in {}",
        output.matches.len(),
        args.query,
        cwd.display()
    );

    if results.is_empty() {
        return Ok(());
    }

    for (idx, result) in results.iter().enumerate() {
        println!(
            "\n{}. [{}] {}",
            idx + 1,
            workspace_source(result.session.source),
            result.session.title
        );
        println!("   dialogue: {}", result.dialogue_title);
        if let Some(timestamp) = result.timestamp {
            println!("   timestamp: {timestamp}");
        }
        println!("   line {}: {}", result.line_index + 1, result.content);
    }

    if output.matches.len() > results.len() {
        println!(
            "\n... {} more match(es). Re-run with --limit {} or --json.",
            output.matches.len() - results.len(),
            output.matches.len()
        );
    }

    Ok(())
}

fn collect_results<'a>(
    sessions: &'a [WorkspaceSession],
    matches: &[WorkspaceSearchMatch],
    limit: usize,
) -> Vec<SearchResult<'a>> {
    matches
        .iter()
        .take(limit)
        .filter_map(|matched| {
            let session = sessions.get(matched.session_index)?;
            let dialogue_title = session
                .dialogue_titles
                .get(matched.dialogue_index)
                .map(String::as_str)
                .unwrap_or("<unknown>");
            let dialogue_index = session
                .original_dialogue_indices
                .get(matched.dialogue_index)
                .copied()
                .unwrap_or(matched.dialogue_index);
            let content = session
                .units
                .get(matched.dialogue_index)
                .and_then(|unit| unit.plain.lines().nth(matched.line_index))
                .unwrap_or(dialogue_title)
                .to_string();
            let timestamp = session
                .unit_timestamps
                .get(matched.dialogue_index)
                .and_then(|timestamp| timestamp.as_deref());
            Some(SearchResult {
                session,
                dialogue_title,
                dialogue_index,
                line_index: matched.line_index,
                timestamp,
                content,
            })
        })
        .collect()
}

fn search_scope(scope: SearchScopeArg) -> WorkspaceSearchScope {
    match scope {
        SearchScopeArg::Content => WorkspaceSearchScope::Content,
        SearchScopeArg::Dialogue => WorkspaceSearchScope::Dialogue,
        SearchScopeArg::Session => WorkspaceSearchScope::Session,
    }
}

fn search_scope_name(scope: WorkspaceSearchScope) -> &'static str {
    match scope {
        WorkspaceSearchScope::Content => "content",
        WorkspaceSearchScope::Session => "session",
        WorkspaceSearchScope::Dialogue => "dialogue",
    }
}
