use anyhow::{Context, Result};
use regex::Regex;
use std::time::SystemTime;

use crate::command_blocks::ParsedCommandBlock as CommandBlock;
use crate::commands::command_block_selector::{parse_selector, resolve_selector, CommandSelection};
use sivtr_core::ai::{
    format_blocks, select_blocks, AgentBlock, AgentBlockKind, AgentProvider, AgentSelection,
    AgentSession, AgentSessionInfo, AgentSessionProvider,
};
use sivtr_core::capture::scrollback;
use sivtr_core::session::{self, SessionEntry};

mod vim;
mod workspace_picker;

use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    TextPair, WorkspaceFocus, WorkspacePickedContent, WorkspaceSession, WorkspaceSource,
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
    plain: CommandBlock,
    ansi: Option<CommandBlock>,
}

impl IndexedCommandBlock {
    fn from_session_entry(entry: &SessionEntry) -> Self {
        let plain = CommandBlock::from_session_entry(entry);
        let ansi = entry.has_ansi().then(|| CommandBlock {
            input_with_prompt: entry.render_input_ansi(),
            input_without_prompt: plain.input_without_prompt.clone(),
            output: entry
                .output_ansi
                .clone()
                .unwrap_or_else(|| plain.output.clone()),
            command: plain.command.clone(),
        });

        Self { plain, ansi }
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
        .map(IndexedCommandBlock::from_session_entry)
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

    let units = build_agent_units(&session, request.selection_mode);
    if units.is_empty() {
        eprintln!("sivtr: selected {provider_name} content is empty");
        return Ok(());
    }

    if request.pick {
        let info = AgentSessionInfo {
            path: path.clone(),
            id: session.id.clone(),
            cwd: session.cwd.clone(),
            modified: std::fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        };
        let choice =
            build_agent_session_choice(source.as_ref(), &info, session, request.selection_mode)
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
    let mut choices = Vec::new();
    for source in sources {
        let sessions = source.list_recent_sessions(None)?;
        choices.extend(build_agent_session_choices(
            source.as_ref(),
            &sessions,
            selection_mode,
        )?);
    }
    choices.sort_by(|a, b| b.modified.cmp(&a.modified));

    let sessions = workspace_sessions_from_agent_choices(choices)?;
    if sessions.is_empty() {
        anyhow::bail!("No terminal or AI sessions with selectable content found");
    }

    run_workspace_picker_on_terminal(terminal, sessions, WorkspaceFocus::Sessions)
}

fn pick_current_agent_sessions_content_on_terminal(
    sources: &[Box<dyn AgentSessionProvider>],
    terminal: &mut crate::tui::terminal::Tui,
    cwd: &std::path::Path,
    selection_mode: AgentSelection,
) -> Result<WorkspacePickedContent> {
    let choices = build_current_agent_session_choices(sources, cwd, selection_mode)?;
    let sessions = workspace_sessions_from_agent_choices(choices)?;
    if sessions.is_empty() {
        anyhow::bail!("No current terminal or AI sessions with selectable content found");
    }

    run_workspace_picker_on_terminal(terminal, sessions, WorkspaceFocus::Sessions)
}

fn build_current_agent_session_choices(
    sources: &[Box<dyn AgentSessionProvider>],
    cwd: &std::path::Path,
    selection_mode: AgentSelection,
) -> Result<Vec<WorkspaceSession>> {
    let mut choices = Vec::new();

    for source in sources {
        let sessions = source.list_recent_sessions(Some(cwd))?;
        choices.extend(build_agent_session_choices(
            source.as_ref(),
            &sessions,
            selection_mode,
        )?);
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
        modified: SystemTime::UNIX_EPOCH,
    };
    let choice =
        build_agent_session_choice(source, &info, session, selection_mode).with_context(|| {
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
        if let Some(choice) = build_agent_session_choice(source, info, session, selection_mode) {
            choices.push(choice);
        }
    }

    Ok(choices)
}

fn build_agent_session_choice(
    source: &dyn AgentSessionProvider,
    info: &AgentSessionInfo,
    session: AgentSession,
    selection_mode: AgentSelection,
) -> Option<WorkspaceSession> {
    let units = build_agent_units(&session, selection_mode);
    if session.blocks.is_empty() || units.is_empty() {
        return None;
    }

    let title = agent_session_display_title(info, &session);
    let dialogue_titles = units
        .iter()
        .map(|unit| build_text_preview(&unit.plain))
        .collect();

    Some(WorkspaceSession {
        source: WorkspaceSource::Agent(source.provider()),
        modified: info.modified,
        title,
        units,
        dialogue_titles,
    })
}

fn workspace_sessions_from_agent_choices(
    mut choices: Vec<WorkspaceSession>,
) -> Result<Vec<WorkspaceSession>> {
    if let Some(session) = build_terminal_context_session()? {
        choices.push(session);
    }
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
    let Some(session) = build_terminal_workspace_session(
        blocks,
        mode,
        include_prompt,
        prompt_override,
        SystemTime::now(),
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

fn build_terminal_context_session() -> Result<Option<WorkspaceSession>> {
    let log_path = scrollback::session_log_path();
    if !log_path.exists() {
        return Ok(None);
    }

    let entries = session::load_entries(&log_path).context("Failed to read session log")?;
    if entries.is_empty() {
        return Ok(None);
    }

    let blocks = entries
        .iter()
        .map(IndexedCommandBlock::from_session_entry)
        .collect::<Vec<_>>();
    Ok(build_terminal_workspace_session(
        &blocks,
        CopyMode::Both,
        true,
        None,
        SystemTime::now(),
    ))
}

fn build_terminal_workspace_session(
    blocks: &[IndexedCommandBlock],
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
    modified: SystemTime,
) -> Option<WorkspaceSession> {
    let entries = blocks
        .iter()
        .filter_map(|block| {
            let unit = format_block_pair(block, mode, include_prompt, prompt_override);
            if unit.plain.trim().is_empty() {
                return None;
            }

            let input = block.plain.input_without_prompt.trim();
            let title = if input.is_empty() {
                build_text_preview(&block.plain.output)
            } else {
                build_text_preview(input)
            };
            Some((unit, title))
        })
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return None;
    }

    let (units, dialogue_titles): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
    let block_count = dialogue_titles.len();
    Some(WorkspaceSession {
        source: WorkspaceSource::Terminal,
        modified,
        title: format!("current shell  [{block_count} blocks]"),
        units,
        dialogue_titles,
    })
}

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
    let plain = format_block(&block.plain, mode, include_prompt, prompt_override);
    let ansi = format_block(
        block.ansi.as_ref().unwrap_or(&block.plain),
        mode,
        include_prompt,
        prompt_override,
    );

    TextPair { plain, ansi }
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

fn format_block(
    block: &CommandBlock,
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> String {
    match mode {
        CopyMode::Both => {
            let input = if include_prompt {
                format_input(block, prompt_override)
            } else {
                block.input_without_prompt.clone()
            };
            match (input.is_empty(), block.output.is_empty()) {
                (false, false) => format!("{}\n{}", input, block.output),
                (false, true) => input,
                (true, false) => block.output.clone(),
                (true, true) => String::new(),
            }
        }
        CopyMode::InputOnly => {
            if include_prompt {
                format_input(block, prompt_override)
            } else {
                block.input_without_prompt.clone()
            }
        }
        CopyMode::OutputOnly => block.output.clone(),
        CopyMode::CommandOnly => block.command.clone(),
    }
}

fn format_input(block: &CommandBlock, prompt_override: Option<&str>) -> String {
    match prompt_override {
        Some(prompt) if !block.command.is_empty() => render_prompt_override(prompt, &block.command),
        Some(_) => block.input_with_prompt.clone(),
        None => block.input_with_prompt.clone(),
    }
}

fn render_prompt_override(prompt: &str, command: &str) -> String {
    let prompt = prompt.trim_end_matches(['\r', '\n']);
    if prompt.is_empty() {
        return command.to_string();
    }

    if prompt.ends_with(' ') || prompt.ends_with('\t') {
        format!("{prompt}{command}")
    } else {
        format!("{prompt} {command}")
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
        if session.blocks.is_empty() || build_agent_units(&session, selection_mode).is_empty() {
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
    let title = agent_session_preview(session)
        .or_else(|| session.id.clone())
        .or_else(|| info.id.clone())
        .unwrap_or_else(|| "<empty AI session>".to_string());
    let id = session
        .id
        .as_deref()
        .or(info.id.as_deref())
        .map(short_agent_id);

    match id {
        Some(id) if !id.is_empty() => format!("{title}  [{id}]"),
        _ => title,
    }
}

fn short_agent_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn is_real_user_block(block: &AgentBlock) -> bool {
    if block.kind != AgentBlockKind::User {
        return false;
    }

    let text = block.text.trim_start();
    !is_agent_startup_user_text(text)
}

fn is_agent_startup_user_text(text: &str) -> bool {
    text.starts_with("# AGENTS.md instructions for")
        || text.starts_with("<environment_context>")
        || text.starts_with("<turn_aborted>")
        || text.starts_with("<local-command-caveat>")
        || text.starts_with("<local-command-stdout>")
        || text.starts_with("<command-message>")
        || text.starts_with("<command-name>")
        || text.starts_with("<ide_opened_file>")
        || text.starts_with("[Request interrupted by user]")
}

fn preview_line(text: &str, limit: usize) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(limit).collect())
}

fn build_agent_units(session: &AgentSession, selection_mode: AgentSelection) -> Vec<TextPair> {
    match selection_mode {
        AgentSelection::LastTurn => build_agent_turn_units(session),
        AgentSelection::LastAssistant => build_agent_kind_units(session, AgentBlockKind::Assistant),
        AgentSelection::LastUser => build_agent_kind_units(session, AgentBlockKind::User),
        AgentSelection::LastTool => build_agent_kind_units(session, AgentBlockKind::ToolOutput),
        AgentSelection::LastBlocks(count) => vec![TextPair {
            plain: format_blocks(&select_blocks(session, AgentSelection::LastBlocks(count))),
            ansi: String::new(),
        }],
        AgentSelection::All => vec![TextPair {
            plain: format_blocks(&session.blocks),
            ansi: String::new(),
        }],
    }
}

fn build_agent_turn_units(session: &AgentSession) -> Vec<TextPair> {
    let mut turns = Vec::new();

    for (start, end) in agent_turn_ranges(&session.blocks) {
        let turn_blocks: Vec<AgentBlock> = session.blocks[start..end]
            .iter()
            .filter(|block| matches!(block.kind, AgentBlockKind::User | AgentBlockKind::Assistant))
            .cloned()
            .collect();

        let text = format_blocks(&turn_blocks);
        if !text.trim().is_empty() {
            turns.push(TextPair {
                plain: text,
                ansi: String::new(),
            });
        }
    }

    turns
}

fn agent_turn_ranges(blocks: &[AgentBlock]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    let mut has_assistant = false;

    for (idx, block) in blocks.iter().enumerate() {
        if block.kind == AgentBlockKind::User && is_real_user_block(block) {
            if let Some(start) = start {
                if has_assistant {
                    ranges.push((start, idx));
                }
            }
            start = Some(idx);
            has_assistant = false;
        } else if start.is_some() && block.kind == AgentBlockKind::Assistant {
            has_assistant = true;
        }
    }

    if let Some(start) = start {
        if has_assistant {
            ranges.push((start, blocks.len()));
        }
    }

    ranges
}

fn build_agent_kind_units(session: &AgentSession, kind: AgentBlockKind) -> Vec<TextPair> {
    session
        .blocks
        .iter()
        .filter(|block| block.kind == kind)
        .map(|block| TextPair {
            plain: block.text.clone(),
            ansi: String::new(),
        })
        .collect()
}

pub(super) fn build_text_preview(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## "))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::vim::{is_vim_command, vim_single_quote};
    use super::{
        agent_session_preview, build_agent_units, build_current_agent_session_choices,
        filter_lines_by_regex, filter_lines_by_spec, format_block, resolve_agent_session_selector,
        AgentBlock, AgentBlockKind, AgentProvider, AgentSelection, AgentSession, AgentSessionInfo,
        AgentSessionProvider, CommandBlock, CopyMode, TextPair, WorkspaceSession,
    };
    use anyhow::Result;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    #[test]
    fn formats_modes() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> git status --all -a".to_string(),
            input_without_prompt: "git status --all -a".to_string(),
            output: "clean".to_string(),
            command: "git status --all -a".to_string(),
        };
        assert_eq!(
            format_block(&block, CopyMode::Both, false, None),
            "git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true, None),
            "PS C:\\repo> git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, false, None),
            "git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true, None),
            "PS C:\\repo> git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::OutputOnly, false, None),
            "clean"
        );
        assert_eq!(
            format_block(&block, CopyMode::CommandOnly, false, None),
            "git status --all -a"
        );
    }

    #[test]
    fn rewrites_prompt_in_copied_input() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> cargo test".to_string(),
            input_without_prompt: "cargo test".to_string(),
            output: "ok".to_string(),
            command: "cargo test".to_string(),
        };

        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true, Some(":")),
            ": cargo test"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true, Some(">>>")),
            ">>> cargo test\nok"
        );
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

        let units = build_agent_units(&session, AgentSelection::LastTurn);

        assert_eq!(units.len(), 1);
        assert!(units[0].plain.contains("review the project"));
        assert!(units[0].plain.contains("first observation"));
        assert!(units[0].plain.contains("final review"));
        assert!(!units[0].plain.contains("Cargo.toml"));
    }

    #[test]
    fn current_agent_picker_lists_all_sessions_for_cwd() {
        let cwd = PathBuf::from("d:\\repo");
        let source = FakeAgentSource {
            require_cwd: true,
            infos: vec![
                AgentSessionInfo {
                    path: PathBuf::from("old.jsonl"),
                    id: Some("old".to_string()),
                    cwd: Some(cwd.display().to_string()),
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                },
                AgentSessionInfo {
                    path: PathBuf::from("new.jsonl"),
                    id: Some("new".to_string()),
                    cwd: Some(cwd.display().to_string()),
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
            ],
        };
        let sources: Vec<Box<dyn AgentSessionProvider>> = vec![Box::new(source)];

        let choices =
            build_current_agent_session_choices(&sources, &cwd, AgentSelection::LastTurn).unwrap();

        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].title, "new task  [new]");
        assert_eq!(choices[1].title, "old task  [old]");
    }

    #[test]
    fn current_agent_picker_does_not_truncate_large_session_lists() {
        let cwd = PathBuf::from("d:\\repo");
        let infos = (0..60)
            .map(|idx| AgentSessionInfo {
                path: PathBuf::from(format!("session-{idx}.jsonl")),
                id: Some(format!("s{idx}")),
                cwd: Some(cwd.display().to_string()),
                modified: SystemTime::UNIX_EPOCH + Duration::from_secs((idx + 1) as u64),
            })
            .collect();
        let source = FakeAgentSource {
            require_cwd: true,
            infos,
        };
        let sources: Vec<Box<dyn AgentSessionProvider>> = vec![Box::new(source)];

        let choices =
            build_current_agent_session_choices(&sources, &cwd, AgentSelection::LastTurn).unwrap();

        assert_eq!(choices.len(), 60);
        assert_eq!(choices[0].title, "session-59 task  [session-]");
        assert_eq!(choices[59].title, "session-0 task  [session-]");
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
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
                AgentSessionInfo {
                    path: PathBuf::from("old.jsonl"),
                    id: Some("old".to_string()),
                    cwd: Some("d:\\repo".to_string()),
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
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
                AgentSessionInfo {
                    path: PathBuf::from("older-valid.jsonl"),
                    id: Some("older-valid".to_string()),
                    cwd: Some("d:\\repo".to_string()),
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
    fn codex_session_picker_uses_first_real_user_message() {
        let session = AgentSession {
            path: "rollout.jsonl".into(),
            id: Some("abc".to_string()),
            cwd: Some("d:\\repo".to_string()),
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
