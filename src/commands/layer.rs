use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::{LayerAction, LayerCommand, LayerOutputFlags};
use crate::commands::capture_history;
use sivtr_core::ai::{AgentProvider, AgentSessionProvider};
use sivtr_core::history::layer::LayerPath;
use sivtr_core::history::HistoryStore;

pub fn execute(cmd: LayerCommand) -> Result<()> {
    let store = HistoryStore::open_default()?;

    match cmd.action {
        LayerAction::Sync { provider, cwd } => {
            sync_all(&store, provider.as_deref(), cwd.as_deref())
        }
        LayerAction::Workspaces(args) => list_workspaces(&store, &args.output),
        LayerAction::Sources(args) => {
            list_sources(&store, &resolve_workspace(args.workspace)?, &args.output)
        }
        LayerAction::Sessions(args) => list_sessions(
            &store,
            &resolve_workspace(args.workspace)?,
            &args.source,
            &args.output,
        ),
        LayerAction::Dialogues(args) => list_dialogues(
            &store,
            &resolve_workspace(args.workspace)?,
            &args.source,
            &args.session,
            &args.output,
        ),
        LayerAction::Content(args) => show_content(
            &store,
            &resolve_workspace(args.workspace)?,
            &args.source,
            &args.session,
            &args.dialogue,
        ),
        LayerAction::Query(args) => query_layer(
            &store,
            &args.keyword,
            &args.scope,
            args.source.as_deref(),
            args.limit,
            &args.output,
        ),
        LayerAction::Tree(args) => {
            show_tree(&store, &resolve_workspace(args.workspace)?, args.depth, &args.output)
        }
        LayerAction::Show(args) => show_entry(&store, &args.table, args.id),
    }
}

fn resolve_workspace(workspace: Option<String>) -> Result<String> {
    if let Some(name) = workspace {
        return Ok(name);
    }
    std::env::current_dir()
        .context("Failed to resolve current directory")?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("Current directory is not valid UTF-8"))
}

/// Resolve a session prefix to the full session_id stored in the database.
fn resolve_session_id(
    store: &HistoryStore,
    workspace: &str,
    source: &str,
    session_prefix: &str,
) -> Result<String> {
    let sessions = store.list_sessions(workspace, source)?;
    // Exact match first
    if sessions.iter().any(|s| s.id == session_prefix) {
        return Ok(session_prefix.to_string());
    }
    // Prefix match
    sessions
        .iter()
        .find(|s| s.id.starts_with(session_prefix))
        .map(|s| s.id.clone())
        .with_context(|| {
            format!(
                "No session matching `{session_prefix}` in `{workspace}/{source}`.\n\
                 Run `sivtr layer sessions {source} -w {workspace} -n` to list sessions."
            )
        })
}

/// Build a stable ref string: `{source}/{session_ref}/{dialogue_id}`
fn session_ref(session_id: &str) -> &str {
    &session_id[..session_id.len().min(8)]
}

fn ref_for_session(source: &str, session_id: &str) -> String {
    format!("{}/{}", source, session_ref(session_id))
}

fn ref_for_dialogue(source: &str, session_id: &str, dialogue_id: &str) -> String {
    format!("{}/{}/{}", source, session_ref(session_id), dialogue_id)
}

// ── compact output helpers ──

/// Emit one result line. Respects -n, -t, --json flags.
struct Line {
    seq: usize,
    ref_: String,
    summary: String,
    timestamp: Option<String>,
    content_preview: Option<String>,
}

#[derive(Serialize)]
struct JsonLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<usize>,
    #[serde(rename = "ref")]
    ref_: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

fn emit_lines(lines: &[Line], flags: &LayerOutputFlags) {
    if flags.json {
        let items: Vec<JsonLine> = lines
            .iter()
            .map(|l| JsonLine {
                n: flags.numbers.then_some(l.seq),
                ref_: l.ref_.clone(),
                summary: l.summary.clone(),
                timestamp: flags.timestamps.then(|| l.timestamp.clone()).flatten(),
                preview: flags.content.then(|| l.content_preview.clone()).flatten(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items).unwrap_or_default());
        return;
    }

    for line in lines {
        let num = if flags.numbers {
            format!("{:>3}. ", line.seq)
        } else {
            String::new()
        };
        let ts = if flags.timestamps {
            line.timestamp
                .as_deref()
                .map(|t| format!("  {}", &t[..t.len().min(19)]))
                .unwrap_or_default()
        } else {
            String::new()
        };
        print!("{num}{}{ts}  {}", line.ref_, line.summary);
        if flags.content {
            if let Some(preview) = &line.content_preview {
                print!("\n    {preview}");
            }
        }
        println!();
    }
}

fn ts(t: &str) -> String {
    t.chars().take(19).collect()
}

// ── list commands ──

fn list_workspaces(store: &HistoryStore, flags: &LayerOutputFlags) -> Result<()> {
    let workspaces = store.list_workspaces()?;
    if workspaces.is_empty() {
        println!("No workspaces found. Run `sivtr layer sync` first.");
        return Ok(());
    }

    let lines: Vec<Line> = workspaces
        .iter()
        .enumerate()
        .map(|(i, ws)| Line {
            seq: i + 1,
            ref_: ws.name.clone(),
            summary: format!(
                "{} sources, {} sessions, {} dialogues",
                ws.source_count, ws.session_count, ws.dialogue_count
            ),
            timestamp: Some(ws.last_active.clone()),
            content_preview: None,
        })
        .collect();

    emit_lines(&lines, flags);
    Ok(())
}

fn list_sources(store: &HistoryStore, workspace: &str, flags: &LayerOutputFlags) -> Result<()> {
    let sources = store.list_sources(workspace)?;
    if sources.is_empty() {
        println!("No sources for `{workspace}`.");
        return Ok(());
    }

    let lines: Vec<Line> = sources
        .iter()
        .enumerate()
        .map(|(i, src)| Line {
            seq: i + 1,
            ref_: src.name.to_string(),
            summary: format!("{} sessions, {} dialogues", src.session_count, src.dialogue_count),
            timestamp: Some(src.last_active.clone()),
            content_preview: None,
        })
        .collect();

    emit_lines(&lines, flags);
    Ok(())
}

fn list_sessions(
    store: &HistoryStore,
    workspace: &str,
    source: &str,
    flags: &LayerOutputFlags,
) -> Result<()> {
    let sessions = store.list_sessions(workspace, source)?;
    if sessions.is_empty() {
        println!("No sessions for `{workspace}/{source}`.");
        return Ok(());
    }

    let lines: Vec<Line> = sessions
        .iter()
        .enumerate()
        .map(|(i, sess)| Line {
            seq: i + 1,
            ref_: ref_for_session(source, &sess.id),
            summary: format!("{} dialogues", sess.dialogue_count),
            timestamp: Some(format!("{} -> {}", ts(&sess.first_at), ts(&sess.last_at))),
            content_preview: None,
        })
        .collect();

    emit_lines(&lines, flags);
    Ok(())
}

fn list_dialogues(
    store: &HistoryStore,
    workspace: &str,
    source: &str,
    session_id: &str,
    flags: &LayerOutputFlags,
) -> Result<()> {
    // Resolve session prefix to full session_id
    let full_id = resolve_session_id(store, workspace, source, session_id)?;
    let dialogues = store.list_dialogues(workspace, source, &full_id)?;
    if dialogues.is_empty() {
        let sref = session_ref(&full_id);
        println!("No dialogues for `{workspace}/{source}/{sref}`.");
        return Ok(());
    }

    let lines: Vec<Line> = dialogues
        .iter()
        .enumerate()
        .map(|(i, dlg)| Line {
            seq: i + 1,
            ref_: ref_for_dialogue(source, session_id, &dlg.id),
            summary: format!("{}i + {}o", dlg.input_count, dlg.output_count),
            timestamp: Some(format!("{} -> {}", ts(&dlg.first_at), ts(&dlg.last_at))),
            content_preview: None,
        })
        .collect();

    emit_lines(&lines, flags);
    Ok(())
}

// ── content (always full) ──

fn show_content(
    store: &HistoryStore,
    workspace: &str,
    source: &str,
    session_id: &str,
    dialogue_id: &str,
) -> Result<()> {
    let full_session_id = resolve_session_id(store, workspace, source, session_id)?;
    let path = LayerPath {
        workspace: workspace.to_string(),
        source: source.to_string(),
        session_id: full_session_id.clone(),
        dialogue_id: dialogue_id.to_string(),
    };

    let inputs = store.list_dialogue_inputs(&path)?;
    let outputs = store.list_dialogue_outputs(&path)?;

    let dlg_ref = ref_for_dialogue(source, &full_session_id, dialogue_id);
    println!("{dlg_ref}  {}i + {}o", inputs.len(), outputs.len());

    for (i, input) in inputs.iter().enumerate() {
        println!(
            "\n── i:{} [{}] {}",
            i + 1,
            input.content_type,
            &input.timestamp[..input.timestamp.len().min(19)]
        );
        println!("{}", input.content);
    }

    for (i, output) in outputs.iter().enumerate() {
        println!(
            "\n── o:{} [{}] {}",
            i + 1,
            output.content_type,
            &output.timestamp[..output.timestamp.len().min(19)]
        );
        println!("{}", output.content);
    }

    Ok(())
}

// ── query ──

fn query_layer(
    store: &HistoryStore,
    keyword: &str,
    scope: &str,
    source_filter: Option<&str>,
    limit: usize,
    flags: &LayerOutputFlags,
) -> Result<()> {
    let search_input = scope == "all" || scope == "input";
    let search_output = scope == "all" || scope == "output";

    let mut all_lines: Vec<Line> = Vec::new();

    if search_input {
        // Search all workspaces, then filter by source in-memory.
        // Scoped search requires exact workspace match which breaks
        // when current_dir() differs from stored workspace paths.
        let results = store.search_input(keyword, limit * 2)?;
        for entry in &results {
            if let Some(src) = source_filter {
                if entry.source != src {
                    continue;
                }
            }
            if all_lines.len() >= limit {
                break;
            }
            let preview: String = entry.content.lines().next().unwrap_or("").chars().take(80).collect();
            all_lines.push(Line {
                seq: 0,
                ref_: format!(
                    "{}/{}/{}",
                    entry.source,
                    session_ref(&entry.session_id),
                    entry.dialogue_id
                ),
                summary: format!("i [{}] {}", entry.content_type, preview),
                timestamp: Some(entry.timestamp.clone()),
                content_preview: Some(preview),
            });
        }
    }

    if search_output && all_lines.len() < limit {
        let results = store.search_output(keyword, limit * 2)?;
        for entry in &results {
            if let Some(src) = source_filter {
                if entry.source != src {
                    continue;
                }
            }
            if all_lines.len() >= limit {
                break;
            }
            let preview: String = entry.content.lines().next().unwrap_or("").chars().take(80).collect();
            all_lines.push(Line {
                seq: 0,
                ref_: format!(
                    "{}/{}/{}",
                    entry.source,
                    session_ref(&entry.session_id),
                    entry.dialogue_id
                ),
                summary: format!("o [{}] {}", entry.content_type, preview),
                timestamp: Some(entry.timestamp.clone()),
                content_preview: Some(preview),
            });
        }
    }

    // Assign sequence numbers after merging
    for (i, line) in all_lines.iter_mut().enumerate() {
        line.seq = i + 1;
    }

    if all_lines.is_empty() {
        println!("No matches for `{keyword}`.");
        return Ok(());
    }

    emit_lines(&all_lines, flags);
    Ok(())
}

// ── tree ──

fn show_tree(
    store: &HistoryStore,
    workspace: &str,
    depth: usize,
    flags: &LayerOutputFlags,
) -> Result<()> {
    let depth = depth.clamp(1, 5);
    println!("{workspace}/");

    if depth < 2 {
        return Ok(());
    }

    let sources = store.list_sources(workspace)?;
    for (src_idx, src) in sources.iter().enumerate() {
        let is_last_src = src_idx == sources.len() - 1;
        let src_prefix = if is_last_src { "└── " } else { "├── " };
        let src_cont = if is_last_src { "    " } else { "│   " };
        let src_label = if flags.numbers {
            format!("{}. {}/", src_idx + 1, src.name)
        } else {
            format!("{}/", src.name)
        };
        println!("{src_prefix}{src_label}");

        if depth < 3 {
            continue;
        }

        let sessions = store.list_sessions(workspace, &src.name)?;
        let session_limit = if depth < 4 { 5.min(sessions.len()) } else { sessions.len() };
        for (sess_idx, sess) in sessions.iter().take(session_limit).enumerate() {
            let is_last_sess = sess_idx == session_limit - 1;
            let sess_prefix = if is_last_sess { "└── " } else { "├── " };
            let sess_cont = if is_last_sess { "    " } else { "│   " };
            let ts_str = if flags.timestamps {
                format!("  {} -> {}", ts(&sess.first_at), ts(&sess.last_at))
            } else {
                String::new()
            };
            println!(
                "{src_cont}{sess_prefix}{}  ({} dialogues){ts_str}",
                ref_for_session(&src.name, &sess.id),
                sess.dialogue_count
            );

            if depth < 4 {
                continue;
            }

            let dialogues = store.list_dialogues(workspace, &src.name, &sess.id)?;
            let dlg_limit = if depth < 5 { 3.min(dialogues.len()) } else { dialogues.len() };
            for (dlg_idx, dlg) in dialogues.iter().take(dlg_limit).enumerate() {
                let is_last_dlg = dlg_idx == dlg_limit - 1;
                let dlg_prefix = if is_last_dlg { "└── " } else { "├── " };
                let dlg_cont = if is_last_dlg { "    " } else { "│   " };
                let ts_str = if flags.timestamps {
                    format!("  {}", ts(&dlg.first_at))
                } else {
                    String::new()
                };
                println!(
                    "{src_cont}{sess_cont}{dlg_prefix}{}  ({}i + {}o){ts_str}",
                    ref_for_dialogue(&src.name, &sess.id, &dlg.id),
                    dlg.input_count,
                    dlg.output_count
                );

                if depth < 5 {
                    continue;
                }

                let path = LayerPath {
                    workspace: workspace.to_string(),
                    source: src.name.clone(),
                    session_id: sess.id.clone(),
                    dialogue_id: dlg.id.clone(),
                };
                let inputs = store.list_dialogue_inputs(&path)?;
                let outputs = store.list_dialogue_outputs(&path)?;
                for (ci, input) in inputs.iter().enumerate() {
                    let preview: String =
                        input.content.lines().next().unwrap_or("").chars().take(60).collect();
                    println!(
                        "{src_cont}{sess_cont}{dlg_cont}├── i:{} [{}] {preview}",
                        ci + 1,
                        input.content_type
                    );
                }
                for (ci, output) in outputs.iter().enumerate() {
                    let preview: String =
                        output.content.lines().next().unwrap_or("").chars().take(60).collect();
                    println!(
                        "{src_cont}{sess_cont}{dlg_cont}├── o:{} [{}] {preview}",
                        ci + 1,
                        output.content_type
                    );
                }
            }

            if sessions.len() > session_limit {
                println!(
                    "{src_cont}... {} more session(s)",
                    sessions.len() - session_limit
                );
            }
        }
    }

    Ok(())
}

// ── sync ──

fn sync_all(
    store: &HistoryStore,
    provider: Option<&str>,
    cwd_override: Option<&str>,
) -> Result<()> {
    eprintln!("Syncing terminal session log...");
    match capture_history::sync_terminal_session_log(store) {
        Ok(n) => eprintln!("  terminal: {n} dialogue(s) synced"),
        Err(e) => eprintln!("  terminal: skipped ({e:#})"),
    }

    let providers: Vec<AgentProvider> = if let Some(name) = provider {
        match AgentProvider::from_command_name(name) {
            Some(p) => vec![p],
            None => {
                eprintln!("Unknown provider `{name}`. Available: codex, claude, opencode, pi");
                return Ok(());
            }
        }
    } else {
        AgentProvider::all()
            .iter()
            .map(|spec| spec.provider)
            .collect()
    };

    for p in &providers {
        eprintln!("Syncing {} sessions...", p.name());
        match sync_agent_sessions(store, *p, cwd_override) {
            Ok(n) => eprintln!("  {}: {n} dialogue(s) synced across all sessions", p.name()),
            Err(e) => eprintln!("  {}: error ({e:#})", p.name()),
        }
    }

    eprintln!("Sync complete. Run `sivtr layer tree` to browse.");
    Ok(())
}

fn sync_agent_sessions(
    store: &HistoryStore,
    provider: AgentProvider,
    cwd_filter: Option<&str>,
) -> Result<usize> {
    let source = provider.session_provider();
    let source_name = provider.command_name();
    let cwd_path = cwd_filter.map(std::path::Path::new);

    let sessions = source.list_recent_sessions(cwd_path)?;
    eprintln!("    {} session(s)", sessions.len());

    let mut total_dialogues = 0;
    for info in &sessions {
        let session_id = info
            .id
            .clone()
            .unwrap_or_else(|| info.path.to_string_lossy().to_string());

        // Use the session's own CWD as workspace, fall back to "unknown"
        let workspace = info
            .cwd
            .as_deref()
            .unwrap_or("unknown");

        if store.is_session_synced(workspace, source_name, &session_id)? {
            continue;
        }

        match source.parse_session_file(&info.path) {
            Ok(session) => {
                match store.sync_agent_session(workspace, source_name, &session_id, &session) {
                    Ok(n) => total_dialogues += n,
                    Err(e) => {
                        eprintln!("    warning: failed to sync {}: {e:#}", info.path.display())
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "    warning: failed to parse {}: {e:#}",
                    info.path.display()
                )
            }
        }
    }

    Ok(total_dialogues)
}

// ── show ──

fn show_entry(store: &HistoryStore, table: &str, id: i64) -> Result<()> {
    match table {
        "i" | "input" => match store.get_input(id)? {
            Some(entry) => {
                println!("── i:#{id} ──");
                println!(
                    "ref:  {}/{}/{}/{}",
                    entry.source,
                    session_ref(&entry.session_id),
                    entry.dialogue_id,
                    entry.content_type
                );
                println!("ts:   {}", entry.timestamp);
                println!("{}", entry.content);
            }
            None => println!("Input entry #{id} not found."),
        },
        "o" | "output" => match store.get_output(id)? {
            Some(entry) => {
                println!("── o:#{id} ──");
                println!(
                    "ref:  {}/{}/{}/{}",
                    entry.source,
                    session_ref(&entry.session_id),
                    entry.dialogue_id,
                    entry.content_type
                );
                println!("ts:   {}", entry.timestamp);
                println!("{}", entry.content);
            }
            None => println!("Output entry #{id} not found."),
        },
        _ => {
            anyhow::bail!("Unknown table `{table}`. Use `i` (input) or `o` (output).");
        }
    }
    Ok(())
}
