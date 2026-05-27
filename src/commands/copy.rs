use anyhow::{Context, Result};
use regex::Regex;
use std::path::Path;
use std::time::SystemTime;

use crate::commands::command_block_selector::{parse_selector, resolve_selector, CommandSelection};
use crate::commands::records::current_work_record_index;
use sivtr_core::ai::{
    AgentBlockKind, AgentProvider, AgentSelection, AgentSession, AgentSessionInfo,
    AgentSessionProvider,
};
use sivtr_core::capture::scrollback;
use sivtr_core::record::{is_real_user_block, RecordTextMode, WorkRecord, WorkRef};
use sivtr_core::session::{self, SessionEntry};

mod vim;
mod workspace_picker;

use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    TextPair, WorkspaceCopyParts, WorkspaceFocus, WorkspacePickedContent, WorkspaceSession,
    WorkspaceSessionLoad, WorkspaceSource,
};
use workspace_picker::run_workspace_picker_on_terminal;

pub(crate) const PICK_CANCELLED_MESSAGE: &str = "Pick cancelled";

pub(crate) fn is_pick_cancelled(error: &anyhow::Error) -> bool {
    error.to_string() == PICK_CANCELLED_MESSAGE
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMode {
    Both,
    InputOnly,
    OutputOnly,
    CommandOnly,
}

#[derive(Clone, Copy, Debug)]
pub struct CopyRequest<'a> {
    pub selector: Option<&'a str>,
    pub pick: bool,
    pub mode: CopyMode,
    pub include_prompt: bool,
    pub prompt_override: Option<&'a str>,
    pub print_full: bool,
    pub ansi: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentCopyRequest<'a> {
    pub provider: AgentProvider,
    pub selector: Option<&'a str>,
    pub session_selector: Option<&'a str>,
    pub pick: bool,
    pub pick_current_session: bool,
    pub selection_mode: AgentSelection,
    pub print_full: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentPickerRequest<'a> {
    pub providers: &'a [AgentProvider],
    pub pick_current_session: bool,
    pub selection_mode: AgentSelection,
    pub print_full: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

fn agent_session_providers(providers: &[AgentProvider]) -> Vec<Box<dyn AgentSessionProvider>> {
    providers
        .iter()
        .copied()
        .map(AgentProvider::session_provider)
        .collect()
}

#[derive(Clone, Debug)]
struct IndexedCommandBlock {
    record: WorkRecord,
}

impl IndexedCommandBlock {
    fn from_session_entry(
        entry: &SessionEntry,
        path: &std::path::Path,
        index: usize,
    ) -> Option<Self> {
        WorkRecord::terminal(entry, path, index).map(|record| Self { record })
    }
}

/// Copy recent command blocks to clipboard.
pub fn execute(request: CopyRequest<'_>) -> Result<()> {
    let CopyRequest {
        selector,
        pick,
        mode,
        include_prompt,
        prompt_override,
        print_full,
        ansi,
        regex,
        lines,
    } = request;

    let log_path = scrollback::session_log_path();
    if !log_path.exists() {
        eprintln!("sivtr: no session log found");
        eprintln!("  hint: run `sivtr init <shell>`, restart the shell, then run some commands");
        return Ok(());
    }

    let entries = session::load_entries(&log_path).context("Failed to read session log")?;
    if entries.is_empty() {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a few commands first, then try `sivtr copy` again");
        return Ok(());
    }

    let blocks: Vec<IndexedCommandBlock> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            IndexedCommandBlock::from_session_entry(entry, &log_path, index)
        })
        .collect();

    let total = blocks.len();
    if total == 0 {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a command first, then try `sivtr copy` again");
        return Ok(());
    }

    if pick {
        return execute_terminal_workspace_pick(
            &blocks,
            mode,
            include_prompt,
            prompt_override,
            print_full,
            ansi,
            regex,
            lines,
        );
    }

    let selection = parse_selector(selector.unwrap_or("1"))?;

    let indices = resolve_selector(selection, total)?;
    if indices.is_empty() {
        eprintln!("sivtr: nothing selected");
        eprintln!("  hint: choose at least one command block");
        return Ok(());
    }

    let copied_blocks: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| blocks.get(*idx))
        .map(|block| format_block_pair(block, mode, include_prompt, prompt_override))
        .filter(|block| !block.plain.trim().is_empty())
        .collect();

    if copied_blocks.is_empty() {
        eprintln!("sivtr: selected commands are empty");
        eprintln!("  hint: try `sivtr copy --out` or choose a different block");
        return Ok(());
    }

    let mut text = join_text_pairs(&copied_blocks, "\n\n");

    if let Some(pattern) = regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    let text = if ansi {
        text.ansi.trim().to_string()
    } else {
        text.plain.trim().to_string()
    };
    finish_copy(
        text,
        print_full,
        format!("sivtr: copied {} command(s) to clipboard", indices.len()),
    )
}

pub fn execute_agent(request: AgentCopyRequest<'_>) -> Result<()> {
    let source = request.provider.session_provider();
    if request.pick && !request.pick_current_session && request.session_selector.is_none() {
        return execute_agent_session_pick(source.as_ref(), request);
    }

    let path = if request.pick && request.pick_current_session && request.session_selector.is_none()
    {
        let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
        match resolve_current_agent_session_with_blocks(source.as_ref(), &cwd)? {
            Some(path) => {
                return execute_current_agent_session_pick(source.as_ref(), request, &path)
            }
            None => return execute_agent_session_pick(source.as_ref(), request),
        }
    } else {
        resolve_agent_session_path(
            source.as_ref(),
            request.session_selector,
            request.pick_current_session,
            request.selection_mode,
        )?
    };
    let session = source.parse_session_file(&path)?;
    let provider_name = source.provider().name();

    if session.blocks.is_empty() {
        eprintln!("sivtr: {provider_name} session has no parsed conversation blocks");
        return Ok(());
    }

    let records =
        WorkRecord::selected_chat_records(source.provider(), &session, request.selection_mode);
    let units = records_to_text_pairs(&records);
    if units.is_empty() {
        eprintln!("sivtr: selected {provider_name} content is empty");
        return Ok(());
    }

    if request.pick {
        let info = AgentSessionInfo {
            path: path.clone(),
            id: session.id.clone(),
            cwd: session.cwd.clone(),
            title: session.title.clone(),
            modified: std::fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        };
        let choice =
            build_agent_session_choice(request.provider, &info, session, request.selection_mode)
                .with_context(|| format!("{provider_name} session has no selectable content"))?;
        let mut terminal = init_tui()?;
        let result = run_workspace_picker_on_terminal(
            &mut terminal,
            vec![choice],
            WorkspaceFocus::Dialogues,
        );
        restore_tui(&mut terminal)?;
        let picked = result?;
        return finish_selected_units_copy(
            &picked.units,
            picked.selection,
            request.print_full,
            request.regex,
            request.lines,
            false,
            format!("selected {provider_name} content is empty"),
            format!("sivtr: copied {provider_name} content to clipboard"),
        );
    }

    let selection = parse_selector(request.selector.unwrap_or("1"))?;
    finish_selected_units_copy(
        &units,
        selection,
        request.print_full,
        request.regex,
        request.lines,
        false,
        format!("selected {provider_name} content is empty"),
        format!("sivtr: copied {provider_name} content to clipboard"),
    )
}

pub fn execute_ref(
    reference: &str,
    cwd: Option<&Path>,
    print_full: bool,
    regex: Option<&str>,
    lines: Option<&str>,
) -> Result<()> {
    let work_ref: WorkRef = reference.parse()?;
    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let providers = work_ref
        .provider()
        .map(|provider| vec![provider])
        .unwrap_or_else(|| {
            AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect()
        });
    let records = current_work_record_index(&providers, &cwd, None)?;
    let record = records
        .resolve(&work_ref)
        .with_context(|| format!("No record found for ref `{reference}`"))?;
    let mut text = ref_text_pair(record, &work_ref, reference)?;

    if let Some(pattern) = regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    finish_copy(
        text.plain.trim().to_string(),
        print_full,
        "sivtr: copied ref content to clipboard".to_string(),
    )
}

fn ref_text_pair(record: &WorkRecord, work_ref: &WorkRef, input_ref: &str) -> Result<TextPair> {
    let plain = record
        .content_for_target(work_ref.target())
        .map(str::to_string)
        .with_context(|| missing_ref_content_message(work_ref, input_ref))?;
    Ok(TextPair {
        ansi: plain.clone(),
        plain,
    })
}

pub fn execute_agent_picker(request: AgentPickerRequest<'_>) -> Result<()> {
    let sources = agent_session_providers(request.providers);
    if sources.is_empty() {
        anyhow::bail!("No AI providers configured for picker");
    }

    let mut terminal = init_tui()?;
    let result = if request.pick_current_session {
        let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
        pick_current_agent_sessions_content_on_terminal(
            &sources,
            &mut terminal,
            &cwd,
            request.selection_mode,
        )
    } else {
        pick_agent_sessions_content_on_terminal(&sources, &mut terminal, request.selection_mode)
    };
    restore_tui(&mut terminal)?;
    let picked = result?;
    match picked.source {
        WorkspaceSource::Agent(provider) => finish_selected_units_copy(
            &picked.units,
            picked.selection,
            request.print_full,
            request.regex,
            request.lines,
            false,
            format!("selected {} content is empty", provider.name()),
            format!("sivtr: copied {} content to clipboard", provider.name()),
        ),
        WorkspaceSource::Terminal => finish_selected_units_copy(
            &picked.units,
            picked.selection,
            request.print_full,
            request.regex,
            request.lines,
            false,
            "selected terminal content is empty".to_string(),
            "sivtr: copied terminal content to clipboard".to_string(),
        ),
    }
}

fn execute_agent_session_pick(
    source: &dyn AgentSessionProvider,
    request: AgentCopyRequest<'_>,
) -> Result<()> {
    let mut terminal = init_tui()?;
    let result =
        pick_agent_session_content_on_terminal(source, &mut terminal, request.selection_mode);
    restore_tui(&mut terminal)?;
    let picked = result?;
    finish_selected_units_copy(
        &picked.units,
        picked.selection,
        request.print_full,
        request.regex,
        request.lines,
        false,
        format!("selected {} content is empty", request.provider.name()),
        format!(
            "sivtr: copied {} content to clipboard",
            request.provider.name()
        ),
    )
}

fn execute_current_agent_session_pick(
    source: &dyn AgentSessionProvider,
    request: AgentCopyRequest<'_>,
    path: &std::path::Path,
) -> Result<()> {
    let mut terminal = init_tui()?;
    let result = pick_current_agent_session_content_on_terminal(
        source,
        &mut terminal,
        path,
        request.selection_mode,
    );
    restore_tui(&mut terminal)?;
    let picked = result?;
    finish_selected_units_copy(
        &picked.units,
        picked.selection,
        request.print_full,
        request.regex,
        request.lines,
        false,
        format!("selected {} content is empty", request.provider.name()),
        format!(
            "sivtr: copied {} content to clipboard",
            request.provider.name()
        ),
    )
}

fn pick_agent_session_content_on_terminal(
    source: &dyn AgentSessionProvider,
    terminal: &mut crate::tui::terminal::Tui,
    selection_mode: AgentSelection,
) -> Result<WorkspacePickedContent> {
    let sessions = source.list_recent_sessions(None)?;
    let choices = build_agent_session_choices(source, &sessions, selection_mode)?;
    if choices.is_empty() {
        anyhow::bail!(
            "No {} sessions with selectable content found",
            source.provider().name()
        );
    }
    run_workspace_picker_on_terminal(terminal, choices, WorkspaceFocus::Sessions)
}

fn pick_agent_sessions_content_on_terminal(
    sources: &[Box<dyn AgentSessionProvider>],
    terminal: &mut crate::tui::terminal::Tui,
    selection_mode: AgentSelection,
) -> Result<WorkspacePickedContent> {
    let mut sessions = Vec::new();
    for source in sources {
        sessions.extend(build_lazy_agent_session_choices(
            source.as_ref(),
            source.list_recent_sessions(None)?,
            selection_mode,
        ));
    }
    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));

    let sessions = workspace_sessions_from_agent_choices(sessions, None)?;
    if sessions.is_empty() {
        anyhow::bail!("No terminal or AI sessions found");
    }

    run_workspace_picker_on_terminal(terminal, sessions, WorkspaceFocus::Sessions)
}

fn pick_current_agent_sessions_content_on_terminal(
    sources: &[Box<dyn AgentSessionProvider>],
    terminal: &mut crate::tui::terminal::Tui,
    cwd: &std::path::Path,
    selection_mode: AgentSelection,
) -> Result<WorkspacePickedContent> {
    let sessions = build_current_lazy_agent_session_choices(sources, cwd, selection_mode)?;
    let sessions = workspace_sessions_from_agent_choices(sessions, Some(cwd))?;
    if sessions.is_empty() {
        anyhow::bail!("No current terminal or AI sessions found");
    }

    run_workspace_picker_on_terminal(terminal, sessions, WorkspaceFocus::Sessions)
}

fn build_current_lazy_agent_session_choices(
    sources: &[Box<dyn AgentSessionProvider>],
    cwd: &std::path::Path,
    selection_mode: AgentSelection,
) -> Result<Vec<WorkspaceSession>> {
    let mut choices = Vec::new();

    for source in sources {
        choices.extend(build_lazy_agent_session_choices(
            source.as_ref(),
            source.list_recent_sessions(Some(cwd))?,
            selection_mode,
        ));
    }

    choices.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(choices)
}

fn pick_current_agent_session_content_on_terminal(
    source: &dyn AgentSessionProvider,
    terminal: &mut crate::tui::terminal::Tui,
    path: &std::path::Path,
    selection_mode: AgentSelection,
) -> Result<WorkspacePickedContent> {
    let session = source.parse_session_file(path)?;
    let info = AgentSessionInfo {
        path: path.to_path_buf(),
        id: session.id.clone(),
        cwd: session.cwd.clone(),
        title: session.title.clone(),
        modified: SystemTime::UNIX_EPOCH,
    };
    let choice = build_agent_session_choice(source.provider(), &info, session, selection_mode)
        .with_context(|| {
            format!(
                "Current {} session has no selectable content",
                source.provider().name()
            )
        })?;
    run_workspace_picker_on_terminal(terminal, vec![choice], WorkspaceFocus::Dialogues)
}

fn build_agent_session_choices(
    source: &dyn AgentSessionProvider,
    sessions: &[AgentSessionInfo],
    selection_mode: AgentSelection,
) -> Result<Vec<WorkspaceSession>> {
    let mut choices = Vec::new();

    for info in sessions {
        let session = source.parse_session_file(&info.path)?;
        if let Some(choice) =
            build_agent_session_choice(source.provider(), info, session, selection_mode)
        {
            choices.push(choice);
        }
    }

    Ok(choices)
}

fn build_lazy_agent_session_choices(
    source: &dyn AgentSessionProvider,
    sessions: Vec<AgentSessionInfo>,
    selection_mode: AgentSelection,
) -> Vec<WorkspaceSession> {
    sessions
        .into_iter()
        .map(|info| build_lazy_agent_session_choice(source.provider(), info, selection_mode))
        .collect()
}

fn build_lazy_agent_session_choice(
    provider: AgentProvider,
    info: AgentSessionInfo,
    selection_mode: AgentSelection,
) -> WorkspaceSession {
    let title = agent_session_info_display_title(&info);
    let search_title = info
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| agent_session_info_fallback_title(&info));

    WorkspaceSession {
        source: WorkspaceSource::Agent(provider),
        modified: info.modified,
        title,
        search_title,
        notice: None,
        records: Vec::new(),
        load: Some(WorkspaceSessionLoad {
            provider,
            path: info.path,
            id: info.id,
            cwd: info.cwd,
            title: info.title,
            modified: info.modified,
            selection_mode,
        }),
    }
}

fn load_workspace_session(session: &WorkspaceSession) -> Result<WorkspaceSession> {
    let Some(load) = &session.load else {
        return Ok(session.clone());
    };

    let info = AgentSessionInfo {
        path: load.path.clone(),
        id: load.id.clone(),
        cwd: load.cwd.clone(),
        title: load.title.clone(),
        modified: load.modified,
    };
    let source = load.provider.session_provider();
    let parsed = match source.parse_session_file(&load.path) {
        Ok(parsed) => parsed,
        Err(_) => {
            return Ok(unavailable_workspace_session(
                load.provider,
                info,
                format!("Failed to load {} session.", load.provider.name()),
            ));
        }
    };

    Ok(resolved_workspace_session(
        load.provider,
        &info,
        parsed,
        load.selection_mode,
    ))
}

pub(super) fn load_workspace_session_at(
    sessions: &mut [WorkspaceSession],
    idx: usize,
) -> Result<()> {
    let Some(session) = sessions.get(idx) else {
        return Ok(());
    };
    if session.load.is_none() {
        return Ok(());
    }
    let loaded = load_workspace_session(session)?;
    if let Some(slot) = sessions.get_mut(idx) {
        *slot = loaded;
    }
    Ok(())
}

pub(super) fn load_workspace_sessions_for_indices(
    sessions: &mut [WorkspaceSession],
    indices: &[usize],
) -> Result<()> {
    for idx in indices {
        load_workspace_session_at(sessions, *idx)?;
    }
    Ok(())
}

fn build_agent_session_choice(
    provider: AgentProvider,
    info: &AgentSessionInfo,
    session: AgentSession,
    selection_mode: AgentSelection,
) -> Option<WorkspaceSession> {
    let records = WorkRecord::selected_chat_records(provider, &session, selection_mode);
    if session.blocks.is_empty() || records.is_empty() {
        return None;
    }

    let title = agent_session_display_title(info, &session);
    let search_title = agent_session_search_title(info, &session);

    Some(WorkspaceSession {
        source: WorkspaceSource::Agent(provider),
        modified: info.modified,
        title,
        search_title,
        notice: None,
        records,
        load: None,
    })
}

fn resolved_workspace_session(
    provider: AgentProvider,
    info: &AgentSessionInfo,
    session: AgentSession,
    selection_mode: AgentSelection,
) -> WorkspaceSession {
    build_agent_session_choice(provider, info, session, selection_mode).unwrap_or_else(|| {
        unavailable_workspace_session(
            provider,
            info.clone(),
            format!("{} session has no selectable content.", provider.name()),
        )
    })
}

fn unavailable_workspace_session(
    provider: AgentProvider,
    info: AgentSessionInfo,
    notice: String,
) -> WorkspaceSession {
    WorkspaceSession {
        source: WorkspaceSource::Agent(provider),
        modified: info.modified,
        title: agent_session_info_display_title(&info),
        search_title: info
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| agent_session_info_fallback_title(&info)),
        notice: Some(notice),
        records: Vec::new(),
        load: None,
    }
}

fn workspace_sessions_from_agent_choices(
    mut choices: Vec<WorkspaceSession>,
    cwd: Option<&std::path::Path>,
) -> Result<Vec<WorkspaceSession>> {
    choices.extend(build_terminal_context_sessions(cwd)?);
    choices.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(choices)
}

#[allow(clippy::too_many_arguments)]
fn execute_terminal_workspace_pick(
    blocks: &[IndexedCommandBlock],
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
    print_full: bool,
    ansi: bool,
    regex: Option<&str>,
    lines: Option<&str>,
) -> Result<()> {
    let session_title = scrollback::workspace_session_log_path()
        .ok()
        .flatten()
        .unwrap_or_else(scrollback::session_log_path);
    let Some(session) = build_terminal_workspace_session(
        blocks,
        mode,
        include_prompt,
        prompt_override,
        SystemTime::now(),
        &sivtr_core::workspace::terminal_session_id_from_path(&session_title),
    ) else {
        eprintln!("sivtr: selected commands are empty");
        eprintln!("  hint: try `sivtr copy --out` or choose a different block");
        return Ok(());
    };

    let mut terminal = init_tui()?;
    let result =
        run_workspace_picker_on_terminal(&mut terminal, vec![session], WorkspaceFocus::Dialogues);
    restore_tui(&mut terminal)?;
    let picked = result?;

    finish_selected_units_copy(
        &picked.units,
        picked.selection,
        print_full,
        regex,
        lines,
        ansi,
        "selected terminal content is empty".to_string(),
        "sivtr: copied terminal content to clipboard".to_string(),
    )
}

fn build_terminal_context_sessions(cwd: Option<&std::path::Path>) -> Result<Vec<WorkspaceSession>> {
    let paths = if let Some(cwd) = cwd {
        sivtr_core::workspace::terminal_log_paths_for_workspace(cwd)?
    } else {
        let path = scrollback::session_log_path();
        if path.exists() {
            vec![path]
        } else {
            Vec::new()
        }
    };

    let mut sessions = Vec::new();
    for log_path in paths {
        let entries = session::load_entries(&log_path).context("Failed to read session log")?;
        if entries.is_empty() {
            continue;
        }

        let blocks = entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                IndexedCommandBlock::from_session_entry(entry, &log_path, index)
            })
            .collect::<Vec<_>>();
        let modified = std::fs::metadata(&log_path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or_else(|_| SystemTime::now());
        if let Some(session) = build_terminal_workspace_session(
            &blocks,
            CopyMode::Both,
            true,
            None,
            modified,
            &sivtr_core::workspace::terminal_session_id_from_path(&log_path),
        ) {
            sessions.push(session);
        }
    }
    Ok(sessions)
}

fn build_terminal_workspace_session(
    blocks: &[IndexedCommandBlock],
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
    modified: SystemTime,
    session_title: &str,
) -> Option<WorkspaceSession> {
    let records = blocks
        .iter()
        .filter_map(|block| {
            let unit = format_block_pair(block, mode, include_prompt, prompt_override);
            (!unit.plain.trim().is_empty()).then(|| block.record.clone())
        })
        .collect::<Vec<_>>();

    if records.is_empty() {
        return None;
    }

    let block_count = records.len();
    let title = format!("{session_title}  [{block_count} blocks]");

    Some(WorkspaceSession {
        source: WorkspaceSource::Terminal,
        modified,
        search_title: title.clone(),
        title,
        notice: None,
        records,
        load: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_selected_units_copy(
    units: &[TextPair],
    selection: CommandSelection,
    print_full: bool,
    regex: Option<&str>,
    lines: Option<&str>,
    ansi: bool,
    empty_message: String,
    success_message: String,
) -> Result<()> {
    let indices = resolve_selector(selection, units.len())?;
    let selected_units: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| units.get(*idx).cloned())
        .filter(|unit| !unit.plain.trim().is_empty())
        .collect();
    if selected_units.is_empty() {
        eprintln!("sivtr: {empty_message}");
        return Ok(());
    }

    let mut text = join_text_pairs(&selected_units, "\n\n");

    if let Some(pattern) = regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    let text = if ansi { text.ansi } else { text.plain };
    finish_copy(text.trim().to_string(), print_full, success_message)
}

fn format_block_pair(
    block: &IndexedCommandBlock,
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> TextPair {
    record_text_to_pair(block.record.copy_text_with_prompt(
        record_text_mode(mode),
        include_prompt,
        prompt_override,
    ))
}

fn join_text_pairs(pairs: &[TextPair], separator: &str) -> TextPair {
    TextPair {
        plain: pairs
            .iter()
            .map(|pair| pair.plain.as_str())
            .collect::<Vec<_>>()
            .join(separator),
        ansi: pairs
            .iter()
            .map(|pair| pair.ansi.as_str())
            .collect::<Vec<_>>()
            .join(separator),
    }
}

fn filter_lines_by_regex(text: &TextPair, pattern: &str) -> Result<TextPair> {
    let regex = Regex::new(pattern)
        .with_context(|| format!("Invalid regex `{pattern}`. Check the pattern syntax."))?;
    let indices = text
        .plain
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| regex.is_match(line).then_some(idx))
        .collect::<Vec<_>>();
    Ok(select_lines(text, &indices))
}

fn filter_lines_by_spec(text: &TextPair, spec: &str) -> Result<TextPair> {
    let lines: Vec<&str> = text.plain.lines().collect();
    let mut selected = Vec::new();

    for part in spec
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let range = part.split_once(':');

        if let Some((start, end)) = range {
            let start = parse_line_number(start)?;
            let end = parse_line_number(end)?;
            if start == 0 || end == 0 {
                anyhow::bail!("Line ranges are 1-based. Example: `10:20`.");
            }
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            for idx in start..=end {
                if lines.get(idx - 1).is_some() {
                    selected.push(idx - 1);
                }
            }
        } else {
            let idx = parse_line_number(part)?;
            if idx == 0 {
                anyhow::bail!("Line numbers are 1-based. Example: `1,3,8:12`.");
            }
            if lines.get(idx - 1).is_some() {
                selected.push(idx - 1);
            }
        }
    }

    Ok(select_lines(text, &selected))
}

fn select_lines(text: &TextPair, indices: &[usize]) -> TextPair {
    let plain_lines: Vec<&str> = text.plain.lines().collect();
    let ansi_lines: Vec<&str> = text.ansi.lines().collect();
    let mut plain_selected = Vec::new();
    let mut ansi_selected = Vec::new();

    for &idx in indices {
        if let Some(line) = plain_lines.get(idx) {
            plain_selected.push((*line).to_string());
            ansi_selected.push(ansi_lines.get(idx).copied().unwrap_or(line).to_string());
        }
    }

    TextPair {
        plain: plain_selected.join("\n"),
        ansi: ansi_selected.join("\n"),
    }
}

fn parse_line_number(value: &str) -> Result<usize> {
    value.parse::<usize>().with_context(|| {
        format!("Invalid line number `{value}`. Use `N`, `A:B`, or comma-separated lists.")
    })
}

fn finish_copy(text: String, print_full: bool, success_message: String) -> Result<()> {
    if text.is_empty() {
        eprintln!("sivtr: filters removed everything");
        eprintln!("  hint: loosen `--regex` or `--lines`, or copy without filters");
        return Ok(());
    }

    sivtr_core::export::clipboard::copy_to_clipboard(&text)?;

    if print_full {
        for line in text.lines() {
            eprintln!("  {line}");
        }
    }

    eprintln!("{success_message}");
    Ok(())
}

fn missing_ref_content_message(work_ref: &WorkRef, input_ref: &str) -> String {
    if let Some((io, index)) = work_ref.part() {
        let label = match io {
            sivtr_core::record::WorkPartIo::Input => "input",
            sivtr_core::record::WorkPartIo::Output => "output",
        };
        format!("No {label} part {index} in ref `{input_ref}`")
    } else if let Some(line) = work_ref.line() {
        format!("No line {line} in ref `{input_ref}`")
    } else {
        format!("No record found for ref `{input_ref}`")
    }
}

fn resolve_agent_session_path(
    source: &dyn AgentSessionProvider,
    session_selector: Option<&str>,
    pick_current_session: bool,
    selection_mode: AgentSelection,
) -> Result<std::path::PathBuf> {
    if let Some(selector) = session_selector {
        return resolve_explicit_agent_session_path(source, selector, selection_mode);
    }
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    if pick_current_session {
        return resolve_current_agent_pick_session_path(source, &cwd);
    }

    source
        .find_current_session(&cwd)?
        .with_context(|| format!("No {} sessions found", source.provider().name()))
}

fn resolve_explicit_agent_session_path(
    source: &dyn AgentSessionProvider,
    selector: &str,
    selection_mode: AgentSelection,
) -> Result<std::path::PathBuf> {
    let sessions = source.list_recent_sessions(None)?;
    resolve_agent_session_selector(source, &sessions, selector, selection_mode)
}

fn resolve_agent_session_selector(
    source: &dyn AgentSessionProvider,
    sessions: &[AgentSessionInfo],
    selector: &str,
    selection_mode: AgentSelection,
) -> Result<std::path::PathBuf> {
    let selector = selector.trim();
    if selector.is_empty() {
        anyhow::bail!(
            "Empty {} session selector. Use `--session 2`, `--session <id>`, or `--pick`.",
            source.provider().name()
        );
    }

    if let Ok(recent) = selector.parse::<usize>() {
        if recent == 0 {
            anyhow::bail!(
                "Session selectors are 1-based. Use `--session 1` for the newest session."
            );
        }
        if !selector.starts_with('0') {
            let selectable = selectable_agent_sessions(source, sessions, selection_mode)?;
            if recent <= selectable.len() {
                return Ok(selectable[recent - 1].path.clone());
            }
        }
    }

    sessions
        .iter()
        .find(|session| agent_session_matches_selector(session, selector))
        .map(|session| session.path.clone())
        .with_context(|| {
            format!(
                "No {} session matched `{selector}`. Use `--pick` to browse recent sessions.",
                source.provider().name()
            )
        })
}

fn selectable_agent_sessions(
    source: &dyn AgentSessionProvider,
    sessions: &[AgentSessionInfo],
    selection_mode: AgentSelection,
) -> Result<Vec<AgentSessionInfo>> {
    let mut selectable = Vec::new();

    for info in sessions {
        let session = source.parse_session_file(&info.path)?;
        if session.blocks.is_empty()
            || WorkRecord::selected_chat_records(source.provider(), &session, selection_mode)
                .is_empty()
        {
            continue;
        }
        selectable.push(info.clone());
    }

    Ok(selectable)
}

fn agent_session_matches_selector(session: &AgentSessionInfo, selector: &str) -> bool {
    session
        .id
        .as_deref()
        .is_some_and(|id| id == selector || id.starts_with(selector))
        || session
            .path
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(selector))
}

fn resolve_current_agent_pick_session_path(
    source: &dyn AgentSessionProvider,
    cwd: &std::path::Path,
) -> Result<std::path::PathBuf> {
    resolve_current_agent_session_with_blocks(source, cwd)?
        .with_context(|| format!("No current {} session found", source.provider().name()))
}

fn resolve_current_agent_session_with_blocks(
    source: &dyn AgentSessionProvider,
    cwd: &std::path::Path,
) -> Result<Option<std::path::PathBuf>> {
    if let Some(path) = current_agent_session_path(source)? {
        return Ok(Some(path));
    }

    for session in source.list_recent_sessions(Some(cwd))? {
        if agent_session_has_blocks(source, &session.path)? {
            return Ok(Some(session.path));
        }
    }

    Ok(None)
}

fn current_agent_session_path(
    source: &dyn AgentSessionProvider,
) -> Result<Option<std::path::PathBuf>> {
    if let Some(path) = current_agent_transcript_path(source.provider()) {
        if agent_session_has_blocks(source, &path)? {
            return Ok(Some(path));
        }
    }

    if let Some(session_id) = current_agent_session_id(source.provider()) {
        if let Some(path) = source.find_session_by_id(&session_id)? {
            if agent_session_has_blocks(source, &path)? {
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

fn current_agent_transcript_path(provider: AgentProvider) -> Option<std::path::PathBuf> {
    let env_name = provider.current_transcript_env()?;

    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

fn current_agent_session_id(provider: AgentProvider) -> Option<String> {
    let env_name = provider.current_session_id_env()?;

    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn agent_session_has_blocks(
    source: &dyn AgentSessionProvider,
    path: &std::path::Path,
) -> Result<bool> {
    Ok(!source.parse_session_file(path)?.blocks.is_empty())
}

fn agent_session_preview(session: &AgentSession) -> Option<String> {
    session
        .blocks
        .iter()
        .find(|block| is_real_user_block(block))
        .and_then(|block| preview_line(&block.text, 80))
        .or_else(|| {
            session
                .blocks
                .iter()
                .find(|block| block.kind == AgentBlockKind::Assistant)
                .and_then(|block| preview_line(&block.text, 80))
        })
}

fn agent_session_display_title(info: &AgentSessionInfo, session: &AgentSession) -> String {
    let title = session
        .title
        .clone()
        .or_else(|| info.title.clone())
        .unwrap_or_else(|| agent_session_fallback_title(info, session));
    agent_session_title_with_id(title, session.id.as_deref().or(info.id.as_deref()))
}

fn agent_session_info_display_title(info: &AgentSessionInfo) -> String {
    agent_session_title_with_id(agent_session_info_fallback_title(info), info.id.as_deref())
}

fn agent_session_title_with_id(title: String, id: Option<&str>) -> String {
    let id = id.map(short_agent_id);

    match id {
        Some(id) if !id.is_empty() => format!("{title}  [{id}]"),
        _ => title,
    }
}

fn agent_session_info_fallback_title(info: &AgentSessionInfo) -> String {
    info.title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| info.id.clone())
        .or_else(|| {
            info.path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "<AI session>".to_string())
}

fn agent_session_fallback_title(info: &AgentSessionInfo, session: &AgentSession) -> String {
    agent_session_preview(session)
        .or_else(|| session.id.clone())
        .or_else(|| info.id.clone())
        .unwrap_or_else(|| "<empty AI session>".to_string())
}

fn agent_session_search_title(info: &AgentSessionInfo, session: &AgentSession) -> String {
    session
        .title
        .clone()
        .or_else(|| info.title.clone())
        .unwrap_or_else(|| agent_session_fallback_title(info, session))
}

fn short_agent_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn preview_line(text: &str, limit: usize) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(limit).collect())
}

fn records_to_text_pairs(records: &[WorkRecord]) -> Vec<TextPair> {
    records
        .iter()
        .map(|record| record_text_to_pair(record.copy_text(RecordTextMode::Combined, false)))
        .collect()
}

pub(crate) fn record_to_copy_parts(
    record: &WorkRecord,
    selection_mode: AgentSelection,
) -> WorkspaceCopyParts {
    match selection_mode {
        AgentSelection::LastBlocks(_) | AgentSelection::All => WorkspaceCopyParts::from_block(
            record_text_to_pair(record.copy_text(RecordTextMode::Combined, false)),
        ),
        _ => WorkspaceCopyParts::from(record.copy_parts(false)),
    }
}

impl From<sivtr_core::record::WorkRecordCopyParts> for WorkspaceCopyParts {
    fn from(parts: sivtr_core::record::WorkRecordCopyParts) -> Self {
        WorkspaceCopyParts {
            input: record_text_to_pair(parts.input),
            output: record_text_to_pair(parts.output),
            block: record_text_to_pair(parts.block),
            command: record_text_to_pair(parts.command),
        }
    }
}

pub(crate) fn record_text_to_pair(text: sivtr_core::record::RecordText) -> TextPair {
    let ansi = text.ansi.unwrap_or_else(|| text.plain.clone());
    TextPair {
        plain: text.plain,
        ansi,
    }
}

fn record_text_mode(mode: CopyMode) -> RecordTextMode {
    match mode {
        CopyMode::Both => RecordTextMode::Combined,
        CopyMode::InputOnly => RecordTextMode::Input,
        CopyMode::OutputOnly => RecordTextMode::Output,
        CopyMode::CommandOnly => RecordTextMode::Command,
    }
}

#[cfg(test)]
mod tests {
    use super::vim::{is_vim_command, vim_single_quote};
    use super::{
        agent_session_preview, filter_lines_by_regex, filter_lines_by_spec, load_workspace_session,
        record_to_copy_parts, records_to_text_pairs, ref_text_pair, resolve_agent_session_selector,
        resolved_workspace_session, AgentBlockKind, AgentProvider, AgentSelection, AgentSession,
        AgentSessionInfo, AgentSessionProvider, TextPair,
    };
    use anyhow::Result;
    use sivtr_core::ai::AgentBlock;
    use sivtr_core::record::{
        RecordTextMode, WorkChannel, WorkOutcome, WorkPart, WorkPartIo, WorkPartKind, WorkPayload,
        WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkStatus, WorkText,
        WorkTime,
    };
    use sivtr_core::session::SessionEntry;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    #[test]
    fn formats_modes() {
        let record = WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "git status --all -a", "clean"),
            Path::new("current"),
            0,
        )
        .unwrap();
        assert_eq!(
            record.copy_text(RecordTextMode::Combined, false).plain,
            "git status --all -a\nclean"
        );
        assert_eq!(
            record.copy_text(RecordTextMode::Combined, true).plain,
            "PS C:\\repo> git status --all -a\nclean"
        );
        assert_eq!(
            record.copy_text(RecordTextMode::Input, false).plain,
            "git status --all -a"
        );
        assert_eq!(
            record.copy_text(RecordTextMode::Input, true).plain,
            "PS C:\\repo> git status --all -a"
        );
        assert_eq!(
            record.copy_text(RecordTextMode::Output, false).plain,
            "clean"
        );
        assert_eq!(
            record.copy_text(RecordTextMode::Command, false).plain,
            "git status --all -a"
        );
    }

    #[test]
    fn rewrites_prompt_in_copied_input() {
        let record = WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "cargo test", "ok"),
            Path::new("current"),
            0,
        )
        .unwrap();

        assert_eq!(
            record
                .copy_text_with_prompt(RecordTextMode::Input, true, Some(":"))
                .plain,
            ": cargo test"
        );
        assert_eq!(
            record
                .copy_text_with_prompt(RecordTextMode::Combined, true, Some(">>>"))
                .plain,
            ">>> cargo test\nok"
        );
    }

    #[test]
    fn resolves_ref_text_for_part_targets() {
        let record = WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "cargo test", "ok"),
            Path::new("current"),
            0,
        )
        .unwrap();
        let reference = WorkRef::terminal_record("current", 1)
            .with_part(sivtr_core::record::WorkPartIo::Output, 1);

        let text = ref_text_pair(&record, &reference, "terminal/current/1/o/1").unwrap();

        assert_eq!(text.plain, "ok");
    }

    #[test]
    fn resolves_ref_text_for_part_refs_emitted_by_work_parts() {
        let record = test_record();
        let reference_text = record
            .session
            .work_ref
            .with_part(WorkPartIo::Output, 1)
            .to_string();
        let reference: WorkRef = reference_text.parse().unwrap();

        let text = ref_text_pair(&record, &reference, &reference_text).unwrap();

        assert_eq!(reference_text, "codex/session/1/o/1");
        assert_eq!(text.plain, "ok");
    }

    #[test]
    fn resolves_ref_text_for_line_targets() {
        let record = test_record();
        let reference = WorkRef::agent_record(AgentProvider::Codex, "session", 1).with_line(2);

        let text = ref_text_pair(&record, &reference, "codex/session/1/2").unwrap();

        assert_eq!(text.plain, "ok");
    }

    #[test]
    fn filters_by_regex() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\nwarn: b\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.plain, "warn: b");
    }

    #[test]
    fn filters_ansi_by_plain_regex_matches() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\n\x1b[31mwarn: b\x1b[0m\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.ansi, "\x1b[31mwarn: b\x1b[0m");
    }

    #[test]
    fn filters_by_line_spec_with_colon_ranges() {
        let filtered = filter_lines_by_spec(
            &TextPair {
                plain: "a\nb\nc\nd".to_string(),
                ansi: "a\nb\nc\nd".to_string(),
            },
            "2,4:3",
        )
        .unwrap();
        assert_eq!(filtered.plain, "b\nc\nd");
    }

    #[test]
    fn rejects_dash_ranges_for_lines() {
        assert!(filter_lines_by_spec(
            &TextPair {
                plain: "a\nb\nc".to_string(),
                ansi: "a\nb\nc".to_string(),
            },
            "1-2"
        )
        .is_err());
    }

    #[test]
    fn detects_vim_compatible_editor_commands() {
        assert!(is_vim_command("nvim"));
        assert!(is_vim_command("vim -Nu NONE"));
        assert!(is_vim_command("C:\\Tools\\gVim\\gvim.exe"));
        assert!(!is_vim_command("code --wait"));
        assert!(!is_vim_command("hx"));
    }

    fn test_record() -> WorkRecord {
        WorkRecord {
            schema_version: 1,
            id: "codex/session/1".to_string(),
            work_ref: WorkRef::agent_record(AgentProvider::Codex, "session", 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "session".to_string(),
                canonical_id: Some("session-0123456789abcdef".to_string()),
                path: None,
                index: 1,
                work_ref: WorkRef::agent_record(AgentProvider::Codex, "session", 1),
            },
            cwd: None,
            time: WorkTime::default(),
            status: WorkStatus {
                outcome: WorkOutcome::Unknown,
                exit_code: None,
            },
            title: "title".to_string(),
            text: WorkText {
                input: Some("user".to_string()),
                output: Some("ok".to_string()),
                combined: "user\nok".to_string(),
            },
            parts: vec![
                WorkPart {
                    io: WorkPartIo::Input,
                    kind: WorkPartKind::UserMessage,
                    index_in_record: 1,
                    occurred_at: None,
                    label: Some("user".to_string()),
                    text: "user".to_string(),
                    ansi: None,
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::AssistantMessage,
                    index_in_record: 1,
                    occurred_at: None,
                    label: Some("assistant".to_string()),
                    text: "ok".to_string(),
                    ansi: None,
                },
            ],
            payload: WorkPayload::ChatTurn {
                user: "user".to_string(),
                assistant: "ok".to_string(),
                messages: Vec::new(),
            },
        }
    }

    #[test]
    fn escapes_vim_single_quoted_strings() {
        assert_eq!(
            vim_single_quote("C:\\tmp\\it's.json"),
            "C:\\tmp\\it''s.json"
        );
    }

    #[test]
    fn agent_turn_units_group_multiple_assistant_messages_for_one_user() {
        let session = AgentSession {
            path: "claude.jsonl".into(),
            id: Some("abc".to_string()),
            cwd: Some("d:\\repo".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    text: "review the project".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolCall,
                    timestamp: None,
                    label: Some("Bash".to_string()),
                    text: "{\"command\":\"rtk ls\"}".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolOutput,
                    timestamp: None,
                    label: Some("Bash".to_string()),
                    text: "Cargo.toml".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: None,
                    label: None,
                    text: "first observation".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: None,
                    label: None,
                    text: "final review".to_string(),
                },
            ],
        };

        let records = sivtr_core::record::WorkRecord::selected_chat_records(
            AgentProvider::Claude,
            &session,
            AgentSelection::LastTurn,
        );
        let units = records_to_text_pairs(&records);

        assert_eq!(units.len(), 1);
        assert!(units[0].plain.contains("review the project"));
        assert!(units[0].plain.contains("first observation"));
        assert!(units[0].plain.contains("final review"));
        assert!(!units[0].plain.contains("Cargo.toml"));
    }

    #[test]
    fn agent_turn_copy_units_strip_role_headings_for_workspace_shortcuts() {
        let session = AgentSession {
            path: "codex.jsonl".into(),
            id: Some("abc".to_string()),
            cwd: Some("d:\\repo".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    text: "question".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: None,
                    label: None,
                    text: "answer".to_string(),
                },
            ],
        };

        let records = sivtr_core::record::WorkRecord::selected_chat_records(
            AgentProvider::Codex,
            &session,
            AgentSelection::LastTurn,
        );
        let copy_units = records
            .iter()
            .map(|record| record_to_copy_parts(record, AgentSelection::LastTurn))
            .collect::<Vec<_>>();

        assert_eq!(copy_units.len(), 1);
        assert_eq!(copy_units[0].input.plain, "question");
        assert_eq!(copy_units[0].output.plain, "answer");
        assert_eq!(copy_units[0].block.plain, "question\n\nanswer");
        assert!(!copy_units[0].block.plain.contains("## Assistant"));
    }

    #[test]
    fn resolved_workspace_session_downgrades_tool_only_sessions_to_notice() {
        let info = AgentSessionInfo {
            path: PathBuf::from("tool-only.jsonl"),
            id: Some("tool-only".to_string()),
            cwd: Some("d:\\repo".to_string()),
            title: Some("tool only".to_string()),
            modified: SystemTime::UNIX_EPOCH,
        };
        let session = AgentSession {
            path: PathBuf::from("tool-only.jsonl"),
            id: Some("tool-only".to_string()),
            cwd: Some("d:\\repo".to_string()),
            title: Some("tool only".to_string()),
            blocks: vec![AgentBlock {
                kind: AgentBlockKind::ToolOutput,
                timestamp: None,
                label: Some("Bash".to_string()),
                text: "tool-only entry".to_string(),
            }],
        };

        let resolved = resolved_workspace_session(
            AgentProvider::Claude,
            &info,
            session,
            AgentSelection::LastTurn,
        );

        assert_eq!(resolved.title, "tool only  [tool-onl]");
        assert_eq!(
            resolved.notice.as_deref(),
            Some("Claude session has no selectable content.")
        );
        assert!(resolved.records.is_empty());
        assert!(resolved.load.is_none());
    }

    #[test]
    fn resolves_agent_session_selector_by_recent_index() {
        let source = FakeAgentSource {
            require_cwd: false,
            infos: vec![
                AgentSessionInfo {
                    path: PathBuf::from("new.jsonl"),
                    id: Some("new".to_string()),
                    cwd: Some("d:\\repo".to_string()),
                    title: None,
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
                AgentSessionInfo {
                    path: PathBuf::from("old.jsonl"),
                    id: Some("old".to_string()),
                    cwd: Some("d:\\repo".to_string()),
                    title: None,
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                },
            ],
        };

        let path =
            resolve_agent_session_selector(&source, &source.infos, "2", AgentSelection::LastTurn)
                .unwrap();

        assert_eq!(path, PathBuf::from("old.jsonl"));
    }

    #[test]
    fn resolves_agent_session_selector_index_uses_selectable_sessions() {
        let source = SparseSelectableSource {
            infos: vec![
                AgentSessionInfo {
                    path: PathBuf::from("new-empty.jsonl"),
                    id: Some("new-empty".to_string()),
                    cwd: Some("d:\\repo".to_string()),
                    title: None,
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
                AgentSessionInfo {
                    path: PathBuf::from("older-valid.jsonl"),
                    id: Some("older-valid".to_string()),
                    cwd: Some("d:\\repo".to_string()),
                    title: None,
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                },
            ],
            sessions: HashMap::from([
                (
                    PathBuf::from("new-empty.jsonl"),
                    AgentSession {
                        path: PathBuf::from("new-empty.jsonl"),
                        id: Some("new-empty".to_string()),
                        cwd: Some("d:\\repo".to_string()),
                        title: None,
                        blocks: vec![AgentBlock {
                            kind: AgentBlockKind::ToolOutput,
                            timestamp: None,
                            label: Some("Bash".to_string()),
                            text: "tool-only entry".to_string(),
                        }],
                    },
                ),
                (
                    PathBuf::from("older-valid.jsonl"),
                    AgentSession {
                        path: PathBuf::from("older-valid.jsonl"),
                        id: Some("older-valid".to_string()),
                        cwd: Some("d:\\repo".to_string()),
                        title: None,
                        blocks: vec![
                            AgentBlock {
                                kind: AgentBlockKind::User,
                                timestamp: None,
                                label: None,
                                text: "question".to_string(),
                            },
                            AgentBlock {
                                kind: AgentBlockKind::Assistant,
                                timestamp: None,
                                label: None,
                                text: "answer".to_string(),
                            },
                        ],
                    },
                ),
            ]),
        };

        let path =
            resolve_agent_session_selector(&source, &source.infos, "1", AgentSelection::LastTurn)
                .unwrap();

        assert_eq!(path, PathBuf::from("older-valid.jsonl"));
    }

    #[test]
    fn resolves_agent_session_selector_by_id_prefix() {
        let source = FakeAgentSource {
            require_cwd: false,
            infos: vec![AgentSessionInfo {
                path: PathBuf::from("rollout-019df7fb.jsonl"),
                id: Some("019df7fb-8289-7fb0-97c3-fe5307ee1b0a".to_string()),
                cwd: Some("d:\\repo".to_string()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            }],
        };

        let path = resolve_agent_session_selector(
            &source,
            &source.infos,
            "019df7fb",
            AgentSelection::LastTurn,
        )
        .unwrap();

        assert_eq!(path, PathBuf::from("rollout-019df7fb.jsonl"));
    }

    #[test]
    fn rejects_zero_agent_session_selector() {
        let source = FakeAgentSource {
            require_cwd: false,
            infos: vec![AgentSessionInfo {
                path: PathBuf::from("only.jsonl"),
                id: Some("only".to_string()),
                cwd: Some("d:\\repo".to_string()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            }],
        };

        let error =
            resolve_agent_session_selector(&source, &source.infos, "0", AgentSelection::LastTurn)
                .unwrap_err();

        assert!(
            error.to_string().contains("Session selectors are 1-based"),
            "{error:#}"
        );
    }

    struct FakeAgentSource {
        require_cwd: bool,
        infos: Vec<AgentSessionInfo>,
    }

    impl AgentSessionProvider for FakeAgentSource {
        fn provider(&self) -> AgentProvider {
            AgentProvider::Codex
        }

        fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
            if self.require_cwd {
                assert!(
                    cwd.is_some(),
                    "current picker must request cwd-filtered sessions"
                );
            }
            Ok(self.infos.clone())
        }

        fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
            let id = path.file_stem().unwrap().to_string_lossy().to_string();
            Ok(AgentSession {
                path: path.to_path_buf(),
                id: Some(id.clone()),
                cwd: Some("d:\\repo".to_string()),
                title: None,
                blocks: vec![
                    AgentBlock {
                        kind: AgentBlockKind::User,
                        timestamp: None,
                        label: None,
                        text: format!("{id} task"),
                    },
                    AgentBlock {
                        kind: AgentBlockKind::Assistant,
                        timestamp: None,
                        label: None,
                        text: "answer".to_string(),
                    },
                ],
            })
        }
    }

    struct SparseSelectableSource {
        infos: Vec<AgentSessionInfo>,
        sessions: HashMap<PathBuf, AgentSession>,
    }

    impl AgentSessionProvider for SparseSelectableSource {
        fn provider(&self) -> AgentProvider {
            AgentProvider::Codex
        }

        fn list_recent_sessions(&self, _cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
            Ok(self.infos.clone())
        }

        fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
            self.sessions
                .get(path)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing session fixture: {}", path.display()))
        }
    }

    #[test]
    fn lazy_workspace_session_with_no_selectable_content_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-02-15T00:00:00Z","type":"session_meta","payload":{"id":"empty","cwd":"d:\\repo"}}
{"timestamp":"2026-02-15T00:00:01Z","type":"response_item","payload":{"type":"function_call","name":"Bash","arguments":"{\"command\":\"echo hi\"}"}}
"#,
        )
        .unwrap();
        let session = super::build_lazy_agent_session_choice(
            AgentProvider::Codex,
            AgentSessionInfo {
                path: path.clone(),
                id: Some("empty".to_string()),
                cwd: Some("d:\\repo".to_string()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
            AgentSelection::LastTurn,
        );

        let loaded = load_workspace_session(&session).unwrap();

        assert!(loaded.records.is_empty());
        assert!(loaded.load.is_none());
        assert_eq!(loaded.title, "empty  [empty]");
    }

    #[test]
    fn codex_session_picker_uses_first_real_user_message() {
        let session = AgentSession {
            path: "rollout.jsonl".into(),
            id: Some("abc".to_string()),
            cwd: Some("d:\\repo".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    text: "# AGENTS.md instructions for d:\\repo\n\n<INSTRUCTIONS>".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    text: "first actual task\nmore details".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: None,
                    label: None,
                    text: "first answer".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    text: "second actual task".to_string(),
                },
            ],
        };

        assert_eq!(
            agent_session_preview(&session).as_deref(),
            Some("first actual task")
        );
    }
}
