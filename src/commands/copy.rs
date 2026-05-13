use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use regex::Regex;
use serde::Serialize;
use std::io::Write;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::command_blocks::CommandBlockSpan;
use crate::command_blocks::ParsedCommandBlock as CommandBlock;
use crate::commands::command_block_selector::{parse_selector, resolve_selector, CommandSelection};
use sivtr_core::ai::{
    format_blocks, select_blocks, AgentBlock, AgentBlockKind, AgentProvider, AgentSelection,
    AgentSession, AgentSessionInfo, AgentSessionProvider,
};
use sivtr_core::capture::scrollback;
use sivtr_core::session::{self, SessionEntry};

mod picker;

use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use picker::{run_picker, run_single_picker, PickEntry};

pub(crate) const PICK_CANCELLED_MESSAGE: &str = "Pick cancelled";
const COMMAND_PICK_LIMIT: usize = 50;
const PICK_PREVIEW_LINES: usize = 8;

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

fn agent_copy_command(provider: AgentProvider) -> String {
    format!("sivtr copy {}", provider.command_name())
}

fn agent_picker_command(providers: &[AgentProvider]) -> String {
    let provider = match providers {
        [provider] => provider.command_name(),
        _ => "all",
    };
    format!("sivtr hotkey-pick-agent --provider {provider}")
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TextPair {
    plain: String,
    ansi: String,
}

#[derive(Clone, Debug)]
enum PickerTuiTarget {
    Unavailable,
    SessionLog,
    Text(VimView),
}

impl PickerTuiTarget {
    fn is_available(&self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

#[derive(Clone, Debug)]
struct VimView {
    raw: String,
    blocks: Vec<VimBlock>,
    alternate: Option<VimAlternateView>,
}

#[derive(Clone, Debug)]
struct VimAlternateView {
    label: String,
    raw: String,
    blocks: Vec<VimBlock>,
}

#[derive(Clone, Debug, Serialize)]
struct VimBlock {
    start: usize,
    end: usize,
    input_start: usize,
    input_end: usize,
    output_start: usize,
    output_end: usize,
    block_text: String,
    input_text: String,
    output_text: String,
    command_text: String,
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

    let selection = if pick {
        pick_selection(&blocks)?
    } else {
        parse_selector(selector.unwrap_or("1"))?
    };

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
    let command = agent_copy_command(source.provider());

    if session.blocks.is_empty() {
        eprintln!("sivtr: {provider_name} session has no parsed conversation blocks");
        return Ok(());
    }

    let units = build_agent_units(&session, request.selection_mode);
    if units.is_empty() {
        eprintln!("sivtr: selected {provider_name} content is empty");
        return Ok(());
    }

    let selection = if request.pick {
        pick_text_selection(
            &units,
            &format!("{command} --pick"),
            build_agent_vim_view(&session.blocks),
        )?
    } else {
        parse_selector(request.selector.unwrap_or("1"))?
    };
    finish_agent_copy(&units, selection, &request)
}

pub fn execute_agent_picker(request: AgentPickerRequest<'_>) -> Result<()> {
    let sources = agent_session_providers(request.providers);
    if sources.is_empty() {
        anyhow::bail!("No AI providers configured for picker");
    }

    let title = agent_picker_command(request.providers);
    let mut terminal = init_tui()?;
    let result = if request.pick_current_session {
        let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
        pick_current_agent_sessions_content_on_terminal(
            &sources,
            &mut terminal,
            &cwd,
            request.selection_mode,
            &title,
        )
    } else {
        pick_agent_sessions_content_on_terminal(
            &sources,
            &mut terminal,
            request.selection_mode,
            &title,
        )
    };
    restore_tui(&mut terminal)?;
    let picked = result?;
    let copy_request = AgentCopyRequest {
        provider: picked.provider,
        selector: None,
        session_selector: None,
        pick: true,
        pick_current_session: request.pick_current_session,
        selection_mode: request.selection_mode,
        print_full: request.print_full,
        regex: request.regex,
        lines: request.lines,
    };
    finish_agent_copy(&picked.units, picked.selection, &copy_request)
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
    finish_agent_copy(&picked.units, picked.selection, &request)
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
    finish_agent_copy(&picked.units, picked.selection, &request)
}

fn pick_agent_session_content_on_terminal(
    source: &dyn AgentSessionProvider,
    terminal: &mut crate::tui::terminal::Tui,
    selection_mode: AgentSelection,
) -> Result<AgentPickedContent> {
    let sessions = source.list_recent_sessions(None)?;
    let choices = build_agent_session_choices(source, &sessions, selection_mode)?;
    if choices.is_empty() {
        anyhow::bail!(
            "No {} sessions with selectable content found",
            source.provider().name()
        );
    }
    run_agent_hierarchy_picker_on_terminal(
        &format!("{} --pick", agent_copy_command(source.provider())),
        terminal,
        agent_groups_from_choices(choices),
        AgentHierarchyFocus::Sessions,
    )
}

fn pick_agent_sessions_content_on_terminal(
    sources: &[Box<dyn AgentSessionProvider>],
    terminal: &mut crate::tui::terminal::Tui,
    selection_mode: AgentSelection,
    title: &str,
) -> Result<AgentPickedContent> {
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

    if choices.is_empty() {
        anyhow::bail!("No AI sessions with selectable content found");
    }

    let groups = agent_groups_from_choices(choices);
    let initial_focus = if groups.len() > 1 {
        AgentHierarchyFocus::Agents
    } else {
        AgentHierarchyFocus::Sessions
    };
    run_agent_hierarchy_picker_on_terminal(title, terminal, groups, initial_focus)
}

fn pick_current_agent_sessions_content_on_terminal(
    sources: &[Box<dyn AgentSessionProvider>],
    terminal: &mut crate::tui::terminal::Tui,
    cwd: &std::path::Path,
    selection_mode: AgentSelection,
    title: &str,
) -> Result<AgentPickedContent> {
    let choices = build_current_agent_session_choices(sources, cwd, selection_mode)?;

    if choices.is_empty() {
        return pick_agent_sessions_content_on_terminal(sources, terminal, selection_mode, title);
    }

    let groups = agent_groups_from_choices(choices);
    let initial_focus = if groups.len() > 1 {
        AgentHierarchyFocus::Agents
    } else {
        AgentHierarchyFocus::Sessions
    };
    run_agent_hierarchy_picker_on_terminal(title, terminal, groups, initial_focus)
}

fn build_current_agent_session_choices(
    sources: &[Box<dyn AgentSessionProvider>],
    cwd: &std::path::Path,
    selection_mode: AgentSelection,
) -> Result<Vec<AgentSessionChoice>> {
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
) -> Result<AgentPickedContent> {
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
    run_agent_hierarchy_picker_on_terminal(
        &format!("{} --pick", agent_copy_command(source.provider())),
        terminal,
        agent_groups_from_choices(vec![choice]),
        AgentHierarchyFocus::Dialogues,
    )
}

#[derive(Clone, Debug)]
struct AgentPickedContent {
    provider: AgentProvider,
    units: Vec<TextPair>,
    selection: CommandSelection,
}

#[derive(Clone, Debug)]
struct AgentGroup {
    provider: AgentProvider,
    title: String,
    choices: Vec<AgentSessionChoice>,
}

#[derive(Clone, Debug)]
struct AgentSessionChoice {
    provider: AgentProvider,
    modified: SystemTime,
    title: String,
    units: Vec<TextPair>,
    dialogue_titles: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentHierarchyFocus {
    Agents,
    Sessions,
    Dialogues,
    Content,
}

struct AgentHierarchyView<'a> {
    groups: &'a [AgentGroup],
    agent_state: &'a ListState,
    session_state: &'a ListState,
    dialogue_state: &'a ListState,
    selected_dialogues: &'a [bool],
    focus: AgentHierarchyFocus,
    content_scroll: usize,
    user_copy_spec: &'a str,
    user_copy_status: Option<&'a str>,
}

fn build_agent_session_choices(
    source: &dyn AgentSessionProvider,
    sessions: &[AgentSessionInfo],
    selection_mode: AgentSelection,
) -> Result<Vec<AgentSessionChoice>> {
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
) -> Option<AgentSessionChoice> {
    let units = build_agent_units(&session, selection_mode);
    if session.blocks.is_empty() || units.is_empty() {
        return None;
    }

    let title = agent_session_display_title(info, &session);
    let dialogue_titles = units
        .iter()
        .rev()
        .map(|unit| build_text_preview(&unit.plain))
        .collect();

    Some(AgentSessionChoice {
        provider: source.provider(),
        modified: info.modified,
        title,
        units,
        dialogue_titles,
    })
}

fn agent_groups_from_choices(choices: Vec<AgentSessionChoice>) -> Vec<AgentGroup> {
    let mut groups: Vec<AgentGroup> = Vec::new();

    for choice in choices {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.provider == choice.provider)
        {
            group.choices.push(choice);
        } else {
            groups.push(AgentGroup {
                provider: choice.provider,
                title: agent_group_title(choice.provider).to_string(),
                choices: vec![choice],
            });
        }
    }

    groups
}

fn agent_group_title(provider: AgentProvider) -> &'static str {
    match provider.command_name() {
        "claude" => "claude code",
        "codex" => "codex",
        _ => provider.command_name(),
    }
}

fn run_agent_hierarchy_picker_on_terminal(
    title: &str,
    terminal: &mut crate::tui::terminal::Tui,
    groups: Vec<AgentGroup>,
    initial_focus: AgentHierarchyFocus,
) -> Result<AgentPickedContent> {
    let mut agent_state = ListState::default();
    agent_state.select(Some(0));
    let mut session_state = ListState::default();
    session_state.select(Some(0));
    let mut dialogue_state = ListState::default();
    dialogue_state.select(Some(0));
    let mut focus = initial_focus;
    let has_agent_layer = initial_focus == AgentHierarchyFocus::Agents;
    let mut selected_dialogues = vec![false; groups[0].choices[0].dialogue_titles.len()];
    let mut content_scroll = 0usize;
    let mut user_copy_spec = String::new();
    let mut user_copy_status: Option<String> = None;

    loop {
        let agent_idx = selected_index(&agent_state).min(groups.len().saturating_sub(1));
        let choices = &groups[agent_idx].choices;
        let session_idx = selected_index(&session_state).min(choices.len().saturating_sub(1));
        let dialogue_count = choices[session_idx].dialogue_titles.len();
        let dialogue_idx = selected_index(&dialogue_state).min(dialogue_count.saturating_sub(1));
        dialogue_state.select((dialogue_count > 0).then_some(dialogue_idx));
        content_scroll = content_scroll.min(
            current_agent_dialogue_text(&choices[session_idx], dialogue_idx)
                .lines()
                .count()
                .saturating_sub(1),
        );

        terminal.draw(|frame| {
            render_agent_hierarchy_picker(
                title,
                frame,
                AgentHierarchyView {
                    groups: &groups,
                    agent_state: &agent_state,
                    session_state: &session_state,
                    dialogue_state: &dialogue_state,
                    selected_dialogues: &selected_dialogues,
                    focus,
                    content_scroll,
                    user_copy_spec: &user_copy_spec,
                    user_copy_status: user_copy_status.as_deref(),
                },
            )
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Esc
                    if can_copy_agent_user_excerpt(focus, dialogue_count)
                        && (!user_copy_spec.is_empty() || user_copy_status.is_some()) =>
                {
                    user_copy_spec.clear();
                    user_copy_status = None;
                }
                KeyCode::Char('q') => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                KeyCode::Esc => match focus {
                    AgentHierarchyFocus::Agents => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                    AgentHierarchyFocus::Sessions if has_agent_layer => {
                        focus = AgentHierarchyFocus::Agents
                    }
                    AgentHierarchyFocus::Sessions => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                    AgentHierarchyFocus::Dialogues => focus = AgentHierarchyFocus::Sessions,
                    AgentHierarchyFocus::Content => focus = AgentHierarchyFocus::Dialogues,
                },
                KeyCode::Left | KeyCode::Char('h') => match focus {
                    AgentHierarchyFocus::Agents => {}
                    AgentHierarchyFocus::Sessions if has_agent_layer => {
                        focus = AgentHierarchyFocus::Agents
                    }
                    AgentHierarchyFocus::Sessions => {}
                    AgentHierarchyFocus::Dialogues => focus = AgentHierarchyFocus::Sessions,
                    AgentHierarchyFocus::Content => focus = AgentHierarchyFocus::Dialogues,
                },
                KeyCode::Right | KeyCode::Char('l') => match focus {
                    AgentHierarchyFocus::Agents => {
                        focus = AgentHierarchyFocus::Sessions;
                    }
                    AgentHierarchyFocus::Sessions if dialogue_count > 0 => {
                        focus = AgentHierarchyFocus::Dialogues;
                    }
                    AgentHierarchyFocus::Dialogues => {
                        focus = AgentHierarchyFocus::Content;
                    }
                    _ => {}
                },
                KeyCode::Up | KeyCode::Char('k') => match focus {
                    AgentHierarchyFocus::Agents => {
                        let next = selected_index(&agent_state).saturating_sub(1);
                        if next != selected_index(&agent_state) {
                            agent_state.select(Some(next));
                            reset_agent_selection_state(
                                &groups,
                                next,
                                &mut session_state,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                            );
                            content_scroll = 0;
                            user_copy_spec.clear();
                            user_copy_status = None;
                        }
                    }
                    AgentHierarchyFocus::Sessions => {
                        let next = selected_index(&session_state).saturating_sub(1);
                        if next != selected_index(&session_state) {
                            session_state.select(Some(next));
                            reset_agent_dialogue_state(
                                choices,
                                next,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                            );
                            content_scroll = 0;
                            user_copy_spec.clear();
                            user_copy_status = None;
                        }
                    }
                    AgentHierarchyFocus::Dialogues => {
                        let next = selected_index(&dialogue_state).saturating_sub(1);
                        dialogue_state.select(Some(next));
                        content_scroll = 0;
                        user_copy_spec.clear();
                        user_copy_status = None;
                    }
                    AgentHierarchyFocus::Content => {
                        content_scroll = content_scroll.saturating_sub(1);
                    }
                },
                KeyCode::Down | KeyCode::Char('j') => match focus {
                    AgentHierarchyFocus::Agents => {
                        let current = selected_index(&agent_state);
                        let next = (current + 1).min(groups.len().saturating_sub(1));
                        if next != current {
                            agent_state.select(Some(next));
                            reset_agent_selection_state(
                                &groups,
                                next,
                                &mut session_state,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                            );
                            content_scroll = 0;
                            user_copy_spec.clear();
                            user_copy_status = None;
                        }
                    }
                    AgentHierarchyFocus::Sessions => {
                        let current = selected_index(&session_state);
                        let next = (current + 1).min(choices.len().saturating_sub(1));
                        if next != current {
                            session_state.select(Some(next));
                            reset_agent_dialogue_state(
                                choices,
                                next,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                            );
                            content_scroll = 0;
                            user_copy_spec.clear();
                            user_copy_status = None;
                        }
                    }
                    AgentHierarchyFocus::Dialogues => {
                        let current = selected_index(&dialogue_state);
                        let next = (current + 1).min(dialogue_count.saturating_sub(1));
                        dialogue_state.select(Some(next));
                        content_scroll = 0;
                        user_copy_spec.clear();
                        user_copy_status = None;
                    }
                    AgentHierarchyFocus::Content => {
                        content_scroll = content_scroll.saturating_add(1);
                    }
                },
                KeyCode::PageDown | KeyCode::Char('d')
                    if focus == AgentHierarchyFocus::Content
                        && (key.code == KeyCode::PageDown
                            || key.modifiers.contains(KeyModifiers::CONTROL)) =>
                {
                    content_scroll = content_scroll.saturating_add(10);
                }
                KeyCode::PageUp | KeyCode::Char('u')
                    if focus == AgentHierarchyFocus::Content
                        && (key.code == KeyCode::PageUp
                            || key.modifiers.contains(KeyModifiers::CONTROL)) =>
                {
                    content_scroll = content_scroll.saturating_sub(10);
                }
                KeyCode::Backspace
                    if can_copy_agent_user_excerpt(focus, dialogue_count)
                        && !user_copy_spec.is_empty() =>
                {
                    user_copy_spec.pop();
                    user_copy_status = None;
                }
                KeyCode::Char(c @ '0'..='9')
                    if can_copy_agent_user_excerpt(focus, dialogue_count) =>
                {
                    user_copy_spec.push(c);
                    user_copy_status = None;
                }
                KeyCode::Char(':')
                    if can_copy_agent_user_excerpt(focus, dialogue_count)
                        && !user_copy_spec.is_empty()
                        && !user_copy_spec.contains(':') =>
                {
                    user_copy_spec.push(':');
                    user_copy_status = None;
                }
                KeyCode::Char('m') if can_copy_agent_user_excerpt(focus, dialogue_count) => {
                    let spec = (!user_copy_spec.is_empty()).then_some(user_copy_spec.as_str());
                    match build_agent_user_excerpt_pick(&choices[session_idx], dialogue_idx, spec) {
                        Ok(picked) => return Ok(picked),
                        Err(err) => user_copy_status = Some(err.to_string()),
                    }
                }
                KeyCode::Char(' ') if focus == AgentHierarchyFocus::Dialogues => {
                    if let Some(selected) = selected_dialogues.get_mut(dialogue_idx) {
                        *selected = !*selected;
                    }
                }
                KeyCode::Char('a') if focus == AgentHierarchyFocus::Dialogues => {
                    let select_all = selected_dialogues.iter().any(|selected| !selected);
                    selected_dialogues.fill(select_all);
                }
                KeyCode::Char('t') if can_open_dialogue_vim(focus, dialogue_count) => {
                    let view = agent_dialogue_vim_view(&choices[session_idx], dialogue_idx);
                    restore_tui(terminal)?;
                    open_vim_view(&view)?;
                    *terminal = init_tui()?;
                }
                KeyCode::Enter => match focus {
                    AgentHierarchyFocus::Agents => {
                        focus = AgentHierarchyFocus::Sessions;
                    }
                    AgentHierarchyFocus::Sessions => {
                        if dialogue_count > 0 {
                            focus = AgentHierarchyFocus::Dialogues;
                        }
                    }
                    AgentHierarchyFocus::Dialogues => {
                        let selection = agent_dialogue_selection(&selected_dialogues, dialogue_idx);
                        return Ok(AgentPickedContent {
                            provider: choices[session_idx].provider,
                            units: choices[session_idx].units.clone(),
                            selection,
                        });
                    }
                    AgentHierarchyFocus::Content => {
                        let selection = agent_dialogue_selection(&selected_dialogues, dialogue_idx);
                        return Ok(AgentPickedContent {
                            provider: choices[session_idx].provider,
                            units: choices[session_idx].units.clone(),
                            selection,
                        });
                    }
                },
                _ => {}
            }
        }
    }
}

fn reset_agent_dialogue_state(
    choices: &[AgentSessionChoice],
    session_idx: usize,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
) {
    dialogue_state.select(Some(0));
    selected_dialogues.clear();
    selected_dialogues.resize(choices[session_idx].dialogue_titles.len(), false);
}

fn reset_agent_selection_state(
    groups: &[AgentGroup],
    agent_idx: usize,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
) {
    session_state.select(Some(0));
    reset_agent_dialogue_state(
        &groups[agent_idx].choices,
        0,
        dialogue_state,
        selected_dialogues,
    );
}

fn agent_dialogue_selection(
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> CommandSelection {
    let mut selected: Vec<usize> = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx + 1))
        .collect();

    if selected.is_empty() {
        selected.push(highlighted_idx + 1);
    }

    CommandSelection::RecentExplicit(selected)
}

fn selected_index(state: &ListState) -> usize {
    state.selected().unwrap_or(0)
}

fn can_open_dialogue_vim(focus: AgentHierarchyFocus, dialogue_count: usize) -> bool {
    dialogue_count > 0
        && matches!(
            focus,
            AgentHierarchyFocus::Sessions
                | AgentHierarchyFocus::Dialogues
                | AgentHierarchyFocus::Content
        )
}

fn can_copy_agent_user_excerpt(focus: AgentHierarchyFocus, dialogue_count: usize) -> bool {
    dialogue_count > 0
        && matches!(
            focus,
            AgentHierarchyFocus::Sessions
                | AgentHierarchyFocus::Dialogues
                | AgentHierarchyFocus::Content
        )
}

fn current_agent_dialogue_text(choice: &AgentSessionChoice, dialogue_idx: usize) -> &str {
    let total = choice.units.len();
    if total == 0 {
        return "<empty>";
    }
    let unit_idx = total.saturating_sub(dialogue_idx + 1);
    choice
        .units
        .get(unit_idx)
        .map(|unit| unit.plain.as_str())
        .unwrap_or("<empty>")
}

fn format_content_with_line_numbers(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let line_count = lines.len().max(1);
    let width = line_count.to_string().len();

    if lines.is_empty() {
        return format!("{:>width$} | ", 1, width = width);
    }

    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{:>width$} | {line}", idx + 1, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_agent_user_excerpt_pick(
    choice: &AgentSessionChoice,
    dialogue_idx: usize,
    spec: Option<&str>,
) -> Result<AgentPickedContent> {
    let user_text = extract_markdown_section(
        current_agent_dialogue_text(choice, dialogue_idx),
        "User",
        "Assistant",
    )?;
    let user_text = match spec {
        Some(spec) => slice_text_by_line_spec(&user_text, spec)?,
        None => user_text,
    };
    let pair = TextPair {
        plain: user_text.clone(),
        ansi: user_text,
    };
    Ok(AgentPickedContent {
        provider: choice.provider,
        units: vec![pair],
        selection: CommandSelection::RecentExplicit(vec![1]),
    })
}

fn extract_markdown_section(text: &str, heading: &str, next_heading: &str) -> Result<String> {
    let mut in_section = false;
    let mut collected = Vec::new();
    for line in text.lines() {
        if !in_section {
            if is_markdown_heading(line, heading) {
                in_section = true;
            }
            continue;
        }
        if is_markdown_heading(line, next_heading) {
            break;
        }
        collected.push(line);
    }
    if !in_section {
        anyhow::bail!("Selected dialogue has no ## {heading} section");
    }
    let section = collected.join("\n").trim().to_string();
    if section.is_empty() {
        anyhow::bail!("Selected dialogue has an empty ## {heading} section");
    }
    Ok(section)
}

fn is_markdown_heading(line: &str, heading: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("##") {
        return false;
    }
    let normalized = trimmed
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .trim();
    normalized.eq_ignore_ascii_case(heading)
}

fn slice_text_by_line_spec(text: &str, spec: &str) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        anyhow::bail!("Selected ## User section is empty");
    }
    let range = parse_user_copy_line_spec(spec, lines.len())?;
    Ok(lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let line_number = idx + 1;
            range.contains(&line_number).then_some(*line)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn parse_user_copy_line_spec(
    spec: &str,
    total_lines: usize,
) -> Result<std::ops::RangeInclusive<usize>> {
    let spec = spec.trim();
    if spec.is_empty() {
        anyhow::bail!("User line range is empty");
    }
    let (start, end) = match spec.split_once(':') {
        Some((start, end)) => (
            parse_positive_line_number(start, "start line")?,
            parse_positive_line_number(end, "end line")?,
        ),
        None => {
            let line = parse_positive_line_number(spec, "line")?;
            (line, line)
        }
    };
    if start > end {
        anyhow::bail!("User line range start must be less than or equal to end");
    }
    if end > total_lines {
        anyhow::bail!("User line range {start}:{end} exceeds {total_lines} available line(s)");
    }
    Ok(start..=end)
}

fn parse_positive_line_number(raw: &str, label: &str) -> Result<usize> {
    let value: usize = raw
        .trim()
        .parse()
        .with_context(|| format!("Invalid {label}: {raw}"))?;
    if value == 0 {
        anyhow::bail!("{label} must be greater than 0");
    }
    Ok(value)
}

fn agent_dialogue_vim_view(choice: &AgentSessionChoice, dialogue_idx: usize) -> VimView {
    let text = current_agent_dialogue_text(choice, dialogue_idx).to_string();
    let end = line_count(&text).max(1);
    VimView {
        blocks: vec![VimBlock {
            start: 1,
            end,
            input_start: 1,
            input_end: end,
            output_start: 1,
            output_end: end,
            block_text: text.clone(),
            input_text: text.clone(),
            output_text: text.clone(),
            command_text: String::new(),
        }],
        alternate: None,
        raw: text,
    }
}

fn render_agent_hierarchy_picker(title: &str, frame: &mut Frame, view: AgentHierarchyView<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let header_height = if view.user_copy_spec.is_empty() && view.user_copy_status.is_none() {
        3
    } else {
        5
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_height), Constraint::Min(1)])
        .split(area);

    let controls = match view.focus {
        AgentHierarchyFocus::Agents => "j/k move  l/Right/Enter open sessions  q/Esc cancel",
        AgentHierarchyFocus::Sessions => {
            "j/k move  l/Right/Enter open dialogues  t open-vim  m copy-user  2:8m copy-user-lines  q/Esc cancel"
        }
        AgentHierarchyFocus::Dialogues => {
            "j/k move  Space toggle  a toggle-all  t open-vim  m copy-user  2:8m copy-user-lines  h/Left/Esc back  Enter copy  q cancel"
        }
        AgentHierarchyFocus::Content => {
            "j/k scroll  Ctrl-d/PageDown down  Ctrl-u/PageUp up  t open-vim  m copy-user  2:8m copy-user-lines  h/Left/Esc back  Enter copy  q cancel"
        }
    };
    let user_copy_state = if view.user_copy_spec.is_empty() && view.user_copy_status.is_none() {
        String::new()
    } else {
        let mut lines = Vec::new();
        if !view.user_copy_spec.is_empty() {
            lines.push(format!(
                "user-copy pending: {}m  (range applies to ## User lines only)",
                view.user_copy_spec
            ));
        }
        if let Some(status) = view.user_copy_status {
            lines.push(status.to_string());
        }
        format!("\n{}", lines.join("\n"))
    };
    let agent_idx = selected_index(view.agent_state).min(view.groups.len().saturating_sub(1));
    let choices = &view.groups[agent_idx].choices;
    let session_idx = selected_index(view.session_state).min(choices.len().saturating_sub(1));
    let selected_count = view
        .selected_dialogues
        .iter()
        .filter(|selected| **selected)
        .count();
    let title = Paragraph::new(format!(
        "{controls}{user_copy_state}\nshowing {} agent(s), {} session(s), {} dialogue(s){}",
        view.groups.len(),
        choices.len(),
        choices[session_idx].dialogue_titles.len(),
        if selected_count == 0 {
            String::new()
        } else {
            format!(", {selected_count} selected")
        }
    ))
    .block(
        Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .title(title),
    );
    frame.render_widget(title, chunks[0]);

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(chunks[1]);

    match view.focus {
        AgentHierarchyFocus::Agents => {
            render_agent_group_list(frame, body_chunks[0], view.groups, view.agent_state, true);
            render_agent_session_list(frame, body_chunks[1], choices, view.session_state, false);
        }
        AgentHierarchyFocus::Sessions => {
            render_agent_session_list(frame, body_chunks[0], choices, view.session_state, true);
            render_agent_dialogue_list(
                frame,
                body_chunks[1],
                &choices[session_idx],
                view.dialogue_state,
                view.selected_dialogues,
                false,
            );
        }
        AgentHierarchyFocus::Dialogues => {
            render_agent_dialogue_list(
                frame,
                body_chunks[0],
                &choices[session_idx],
                view.dialogue_state,
                view.selected_dialogues,
                true,
            );
            let dialogue_idx = selected_index(view.dialogue_state)
                .min(choices[session_idx].dialogue_titles.len().saturating_sub(1));
            let content = Paragraph::new(format_content_with_line_numbers(
                current_agent_dialogue_text(&choices[session_idx], dialogue_idx),
            ))
            .scroll((view.content_scroll as u16, 0))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("Content"));
            frame.render_widget(content, body_chunks[1]);
        }
        AgentHierarchyFocus::Content => {
            render_agent_dialogue_list(
                frame,
                body_chunks[0],
                &choices[session_idx],
                view.dialogue_state,
                view.selected_dialogues,
                false,
            );
            let dialogue_idx = selected_index(view.dialogue_state)
                .min(choices[session_idx].dialogue_titles.len().saturating_sub(1));
            let content = Paragraph::new(format_content_with_line_numbers(
                current_agent_dialogue_text(&choices[session_idx], dialogue_idx),
            ))
            .scroll((view.content_scroll as u16, 0))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("Content *"));
            frame.render_widget(content, body_chunks[1]);
        }
    }
}

fn render_agent_session_list(
    frame: &mut Frame,
    area: Rect,
    choices: &[AgentSessionChoice],
    state: &ListState,
    active: bool,
) {
    let items: Vec<ListItem> = choices
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let line = format!("{:>2}. {}", idx + 1, choice.title);
            ListItem::new(line)
        })
        .collect();
    let title = if active { "Sessions *" } else { "Sessions" };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, area, &mut local_state);
}

fn render_agent_group_list(
    frame: &mut Frame,
    area: Rect,
    groups: &[AgentGroup],
    state: &ListState,
    active: bool,
) {
    let items: Vec<ListItem> = groups
        .iter()
        .enumerate()
        .map(|(idx, group)| ListItem::new(format!("{:>2}. {}", idx + 1, group.title)))
        .collect();
    let title = if active { "Agents *" } else { "Agents" };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(if active {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        })
        .highlight_symbol(if active { ">> " } else { "   " });
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, area, &mut local_state);
}

fn render_agent_dialogue_list(
    frame: &mut Frame,
    area: Rect,
    choice: &AgentSessionChoice,
    state: &ListState,
    selected_dialogues: &[bool],
    active: bool,
) {
    let mut items: Vec<ListItem> = choice
        .dialogue_titles
        .iter()
        .enumerate()
        .map(|(idx, title)| {
            let marker = if active {
                if selected_dialogues.get(idx).copied().unwrap_or(false) {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                ""
            };
            ListItem::new(format!("{marker}{:>2}. {title}", idx + 1))
        })
        .collect();

    if items.is_empty() {
        items.push(ListItem::new("<empty>"));
    }

    let title = if active { "Dialogues *" } else { "Dialogues" };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(if active {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        })
        .highlight_symbol(if active { ">> " } else { "   " });
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, area, &mut local_state);
}

fn finish_agent_copy(
    units: &[TextPair],
    selection: CommandSelection,
    request: &AgentCopyRequest<'_>,
) -> Result<()> {
    let indices = resolve_selector(selection, units.len())?;
    let selected_units: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| units.get(*idx).cloned())
        .filter(|unit| !unit.plain.trim().is_empty())
        .collect();
    if selected_units.is_empty() {
        eprintln!(
            "sivtr: selected {} content is empty",
            request.provider.name()
        );
        return Ok(());
    }

    let mut text = join_text_pairs(&selected_units, "\n\n");

    if let Some(pattern) = request.regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = request.lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    finish_copy(
        text.plain.trim().to_string(),
        request.print_full,
        format!(
            "sivtr: copied {} content to clipboard",
            request.provider.name()
        ),
    )
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

fn pick_selection(blocks: &[IndexedCommandBlock]) -> Result<CommandSelection> {
    let total = blocks.len();
    let shown = total.min(COMMAND_PICK_LIMIT);
    let entries: Vec<PickEntry> = blocks
        .iter()
        .rev()
        .take(shown)
        .enumerate()
        .map(|(offset, block)| {
            let recent = offset + 1;
            let output_preview = build_output_preview(&block.plain);
            let preview = if !block.plain.command.is_empty() {
                block.plain.command.clone()
            } else if !block.plain.output.is_empty() {
                block.plain.output.lines().next().unwrap_or("").to_string()
            } else {
                "<empty>".to_string()
            };
            PickEntry {
                recent,
                preview,
                output_preview,
                full_preview: block.plain.output.clone(),
                selected: false,
            }
        })
        .collect();

    run_picker(
        entries,
        total,
        "sivtr copy --pick",
        PickerTuiTarget::SessionLog,
    )
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
    if let Some(path) = resolve_current_agent_session_with_blocks(source, cwd)? {
        return Ok(path);
    }

    pick_agent_session_path(source, &source.list_recent_sessions(None)?)?
        .with_context(|| format!("No {} sessions found", source.provider().name()))
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

fn pick_agent_session_path(
    source: &dyn AgentSessionProvider,
    sessions: &[AgentSessionInfo],
) -> Result<Option<std::path::PathBuf>> {
    if sessions.is_empty() {
        return Ok(None);
    }

    let entries = build_agent_session_pick_entries(source, sessions)?;

    let selected = run_single_picker(
        entries,
        sessions.len(),
        &format!("{} --session", agent_copy_command(source.provider())),
        PickerTuiTarget::Unavailable,
    )?;
    Ok(sessions
        .get(selected.saturating_sub(1))
        .map(|session| session.path.clone()))
}

fn build_agent_session_pick_entries(
    source: &dyn AgentSessionProvider,
    sessions: &[AgentSessionInfo],
) -> Result<Vec<PickEntry>> {
    sessions
        .iter()
        .enumerate()
        .map(|(idx, session)| build_agent_session_pick_entry(source, idx, session))
        .collect()
}

fn build_agent_session_pick_entry(
    source: &dyn AgentSessionProvider,
    idx: usize,
    session: &AgentSessionInfo,
) -> Result<PickEntry> {
    let parsed = source.parse_session_file(&session.path)?;
    let preview = agent_session_preview(&parsed)
        .or_else(|| session.id.clone())
        .unwrap_or_else(|| format!("<empty {} session>", source.provider().name()));
    let meta = format_agent_session_meta(session, &parsed);

    Ok(PickEntry {
        recent: idx + 1,
        preview,
        output_preview: meta
            .lines()
            .take(PICK_PREVIEW_LINES)
            .collect::<Vec<_>>()
            .join("\n"),
        full_preview: meta,
        selected: false,
    })
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

fn format_agent_session_meta(info: &AgentSessionInfo, session: &AgentSession) -> String {
    let id = session
        .id
        .as_deref()
        .or(info.id.as_deref())
        .unwrap_or("<no id>");
    let cwd = session
        .cwd
        .as_deref()
        .or(info.cwd.as_deref())
        .unwrap_or("<no cwd>");
    format!("id: {id}\ncwd: {cwd}\npath: {}", info.path.display())
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

fn build_agent_vim_view(blocks: &[AgentBlock]) -> VimView {
    let mut main_turns = Vec::new();
    let mut full_turns = Vec::new();

    for (start, end) in agent_turn_ranges(blocks) {
        let full_blocks = &blocks[start..end];
        let main_blocks: Vec<AgentBlock> = full_blocks
            .iter()
            .filter(|block| matches!(block.kind, AgentBlockKind::User | AgentBlockKind::Assistant))
            .cloned()
            .collect();

        if !main_blocks.is_empty() {
            main_turns.push(main_blocks);
            full_turns.push(full_blocks.to_vec());
        }
    }

    let main = build_agent_turn_view(&main_turns);
    let full = build_agent_turn_view(&full_turns);

    VimView {
        raw: main.raw,
        blocks: main.blocks,
        alternate: Some(VimAlternateView {
            label: "tools".to_string(),
            raw: full.raw,
            blocks: full.blocks,
        }),
    }
}

fn build_agent_turn_view(turns: &[Vec<AgentBlock>]) -> VimAlternateView {
    let mut rendered_turns = Vec::new();
    let mut raw_parts = Vec::new();
    let mut next_line = 1usize;

    for turn in turns {
        let rendered_blocks: Vec<String> = turn.iter().map(format_agent_block_for_vim).collect();
        let rendered = rendered_blocks.join("\n\n");
        if rendered.trim().is_empty() {
            continue;
        }

        let line_count = line_count(&rendered);
        let start = next_line;
        let end = start + line_count.saturating_sub(1);
        let input_text = join_agent_kind_text(turn, AgentBlockKind::User);
        let output_text = turn
            .iter()
            .filter(|block| {
                matches!(
                    block.kind,
                    AgentBlockKind::Assistant | AgentBlockKind::ToolOutput
                )
            })
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let command_text = turn
            .iter()
            .filter(|block| block.kind == AgentBlockKind::ToolCall)
            .filter_map(|block| block.label.as_deref())
            .collect::<Vec<_>>()
            .join("\n");

        rendered_turns.push(VimBlock {
            start,
            end,
            input_start: range_for_first_kind(turn, AgentBlockKind::User, start).0,
            input_end: range_for_first_kind(turn, AgentBlockKind::User, start).1,
            output_start: range_for_first_kind(turn, AgentBlockKind::Assistant, start).0,
            output_end: range_for_first_kind(turn, AgentBlockKind::Assistant, start).1,
            block_text: rendered.clone(),
            input_text,
            output_text,
            command_text,
        });
        raw_parts.push(rendered);
        next_line = end + 2;
    }

    VimAlternateView {
        label: "main".to_string(),
        raw: raw_parts.join("\n\n"),
        blocks: rendered_turns,
    }
}

fn join_agent_kind_text(turn: &[AgentBlock], kind: AgentBlockKind) -> String {
    turn.iter()
        .filter(|block| block.kind == kind)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn range_for_first_kind(
    turn: &[AgentBlock],
    kind: AgentBlockKind,
    turn_start_line: usize,
) -> (usize, usize) {
    let mut cursor = turn_start_line;
    for block in turn {
        let rendered = format_agent_block_for_vim(block);
        let count = line_count(&rendered);
        if block.kind == kind {
            let body_start = (cursor + 1).min(cursor + count.saturating_sub(1));
            return (body_start, cursor + count.saturating_sub(1));
        }
        cursor += count + 2;
    }
    (0, 0)
}

fn format_agent_block_for_vim(block: &AgentBlock) -> String {
    let heading = match block.kind {
        AgentBlockKind::User => "User".to_string(),
        AgentBlockKind::Assistant => "Assistant".to_string(),
        AgentBlockKind::ToolCall => block
            .label
            .as_deref()
            .map(|label| format!("Tool Call: {label}"))
            .unwrap_or_else(|| "Tool Call".to_string()),
        AgentBlockKind::ToolOutput => "Tool Output".to_string(),
    };
    format!("## {heading}\n{}", block.text.trim())
}

fn build_session_vim_view(raw: String) -> Result<VimView> {
    let spans = crate::command_blocks::load_from_session_log()?.unwrap_or_default();
    Ok(VimView {
        raw,
        blocks: spans.iter().map(VimBlock::from_command_span).collect(),
        alternate: None,
    })
}

impl VimBlock {
    fn from_command_span(span: &CommandBlockSpan) -> Self {
        let (input_start, input_end) = one_based_range(span.input_line_range);
        let (output_start, output_end) = one_based_range(span.output_line_range);
        Self {
            start: span.line_start + 1,
            end: span.line_end + 1,
            input_start,
            input_end,
            output_start,
            output_end,
            block_text: span
                .text_for(crate::command_blocks::CopyTarget::Block)
                .unwrap_or_default(),
            input_text: span
                .text_for(crate::command_blocks::CopyTarget::Input)
                .unwrap_or_default(),
            output_text: span
                .text_for(crate::command_blocks::CopyTarget::Output)
                .unwrap_or_default(),
            command_text: span
                .text_for(crate::command_blocks::CopyTarget::Command)
                .unwrap_or_default(),
        }
    }
}

fn one_based_range(range: Option<(usize, usize)>) -> (usize, usize) {
    range
        .map(|(start, end)| (start + 1, end + 1))
        .unwrap_or((0, 0))
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn pick_text_selection(
    units: &[TextPair],
    title: &str,
    vim_view: VimView,
) -> Result<CommandSelection> {
    let total = units.len();
    run_picker(
        build_text_pick_entries(units),
        total,
        title,
        PickerTuiTarget::Text(vim_view),
    )
}

fn build_text_pick_entries(units: &[TextPair]) -> Vec<PickEntry> {
    units
        .iter()
        .rev()
        .enumerate()
        .map(|(offset, unit)| PickEntry {
            recent: offset + 1,
            preview: build_text_preview(&unit.plain),
            output_preview: build_text_preview_lines(&unit.plain),
            full_preview: unit.plain.clone(),
            selected: false,
        })
        .collect()
}

fn open_picker_vim(target: &PickerTuiTarget) -> Result<()> {
    let view = match target {
        PickerTuiTarget::Unavailable => {
            return Ok(());
        }
        PickerTuiTarget::SessionLog => match scrollback::read_session_log()? {
            Some(raw) if !raw.trim().is_empty() => build_session_vim_view(raw)?,
            _ => {
                eprintln!("sivtr: session log is empty");
                return Ok(());
            }
        },
        PickerTuiTarget::Text(view) => view.clone(),
    };
    open_vim_view(&view)
}

fn open_vim_view(view: &VimView) -> Result<()> {
    let editor = resolve_vim_editor()?;
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let content_path = dir.join(format!("sivtr-view-{}-{nonce}.txt", std::process::id()));
    let vimrc_path = dir.join(format!("sivtr-view-{}-{nonce}.vim", std::process::id()));
    let blocks_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.blocks.json",
        std::process::id()
    ));
    let alternate_content_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.tools.txt",
        std::process::id()
    ));
    let alternate_blocks_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.tools.blocks.json",
        std::process::id()
    ));

    std::fs::write(&content_path, &view.raw).context("Failed to write Vim view file")?;
    let blocks_json =
        serde_json::to_string(&view.blocks).context("Failed to encode Vim block data")?;
    std::fs::write(&blocks_path, blocks_json).context("Failed to write Vim block data")?;
    let alternate = if let Some(alternate) = &view.alternate {
        std::fs::write(&alternate_content_path, &alternate.raw)
            .context("Failed to write alternate Vim view file")?;
        let blocks_json = serde_json::to_string(&alternate.blocks)
            .context("Failed to encode alternate Vim block data")?;
        std::fs::write(&alternate_blocks_path, blocks_json)
            .context("Failed to write alternate Vim block data")?;
        Some((
            alternate.label.as_str(),
            alternate_content_path.as_path(),
            alternate_blocks_path.as_path(),
        ))
    } else {
        None
    };
    write_vimrc(&vimrc_path, &blocks_path, alternate)?;

    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (program, extra_args) = parts
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Empty Vim editor command"))?;

    let status = Command::new(program)
        .args(extra_args)
        .arg("-u")
        .arg(&vimrc_path)
        .arg("-n")
        .arg("-R")
        .arg(&content_path)
        .status()
        .with_context(|| format!("Failed to launch Vim editor `{editor}`"))?;

    let _ = std::fs::remove_file(&content_path);
    let _ = std::fs::remove_file(&vimrc_path);
    let _ = std::fs::remove_file(&blocks_path);
    let _ = std::fs::remove_file(&alternate_content_path);
    let _ = std::fs::remove_file(&alternate_blocks_path);

    if !status.success() {
        anyhow::bail!("Vim editor `{editor}` exited with {status}");
    }
    Ok(())
}

fn resolve_vim_editor() -> Result<String> {
    let config = sivtr_core::config::SivtrConfig::load().unwrap_or_default();
    if is_vim_command(&config.editor.command) {
        return Ok(config.editor.command);
    }

    for candidate in ["nvim", "vim", "vi"] {
        if command_exists(candidate) {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("No Vim-compatible editor found. Set `editor.command` to nvim/vim/vi.")
}

fn is_vim_command(command: &str) -> bool {
    let Some(program) = command.split_whitespace().next() else {
        return false;
    };
    let name = std::path::Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_lowercase();
    name == "vi" || name.contains("vim")
}

fn vim_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn command_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("which")
            .arg(name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

fn write_vimrc(
    path: &std::path::Path,
    blocks_path: &std::path::Path,
    alternate: Option<(&str, &std::path::Path, &std::path::Path)>,
) -> Result<()> {
    let mut file = std::fs::File::create(path).context("Failed to create temporary Vim config")?;
    let blocks_path = vim_single_quote(&blocks_path.to_string_lossy());
    let (alternate_label, alternate_content_path, alternate_blocks_path) =
        if let Some((label, content, blocks)) = alternate {
            (
                vim_single_quote(label),
                vim_single_quote(&content.to_string_lossy()),
                vim_single_quote(&blocks.to_string_lossy()),
            )
        } else {
            (String::new(), String::new(), String::new())
        };
    let script = format!(
        r#"
set nocompatible
set nomodeline
set readonly
set nomodifiable
set nomodified
set number
set nowrap
set nofoldenable
let s:sivtr_blocks = json_decode(join(readfile('{blocks_path}'), "\n"))
let s:sivtr_main_blocks_path = '{blocks_path}'
let s:sivtr_alt_label = '{alternate_label}'
let s:sivtr_alt_content_path = '{alternate_content_path}'
let s:sivtr_alt_blocks_path = '{alternate_blocks_path}'
let s:sivtr_tools_visible = 0

function! s:SivtrLoadBlocks(path) abort
  let s:sivtr_blocks = json_decode(join(readfile(a:path), "\n"))
endfunction

function! s:SivtrToggleTools() abort
  if empty(s:sivtr_alt_content_path)
    echo 'sivtr: no alternate view'
    return
  endif
  let l:top = winsaveview()
  setlocal modifiable
  silent %delete _
  if s:sivtr_tools_visible
    silent execute '0read ' . fnameescape(expand('%:p'))
    silent 1delete _
    call s:SivtrLoadBlocks(s:sivtr_main_blocks_path)
    let s:sivtr_tools_visible = 0
    echo 'sivtr: tools hidden'
  else
    silent execute '0read ' . fnameescape(s:sivtr_alt_content_path)
    silent 1delete _
    call s:SivtrLoadBlocks(s:sivtr_alt_blocks_path)
    let s:sivtr_tools_visible = 1
    echo 'sivtr: ' . s:sivtr_alt_label . ' visible'
  endif
  setlocal nomodifiable nomodified readonly
  call winrestview(l:top)
endfunction

function! s:SivtrCurrentBlockIndex() abort
  let l:line = line('.')
  let l:previous = -1
  for l:i in range(0, len(s:sivtr_blocks) - 1)
    let l:block = s:sivtr_blocks[l:i]
    if l:line >= l:block.start && l:line <= l:block.end
      return l:i
    endif
    if l:block.start <= l:line
      let l:previous = l:i
    endif
  endfor
  return l:previous >= 0 ? l:previous : 0
endfunction

function! s:SivtrCurrentBlock() abort
  if empty(s:sivtr_blocks)
    echohl ErrorMsg | echo 'sivtr: no blocks' | echohl None
    return {{}}
  endif
  return s:sivtr_blocks[s:SivtrCurrentBlockIndex()]
endfunction

function! s:SivtrJump(delta) abort
  if empty(s:sivtr_blocks)
    echohl ErrorMsg | echo 'sivtr: no blocks' | echohl None
    return
  endif
  let l:idx = s:SivtrCurrentBlockIndex() + a:delta
  let l:idx = max([0, min([l:idx, len(s:sivtr_blocks) - 1])])
  call cursor(s:sivtr_blocks[l:idx].start, 1)
  normal! zz
endfunction

function! s:SivtrCopy(kind) abort
  let l:block = s:SivtrCurrentBlock()
  if empty(l:block)
    return
  endif
  let l:key = a:kind . '_text'
  let l:text = get(l:block, l:key, '')
  if empty(l:text)
    echohl ErrorMsg | echo 'sivtr: current block has no ' . a:kind . ' content' | echohl None
    return
  endif
  call setreg('"', l:text)
  try | call setreg('+', l:text) | catch | endtry
  try | call setreg('*', l:text) | catch | endtry
  echo 'sivtr: copied current ' . a:kind
endfunction

function! s:SivtrSelect(kind) abort
  let l:block = s:SivtrCurrentBlock()
  if empty(l:block)
    return
  endif
  if a:kind ==# 'block'
    let [l:start, l:end] = [l:block.start, l:block.end]
  elseif a:kind ==# 'input'
    let [l:start, l:end] = [l:block.input_start, l:block.input_end]
  else
    let [l:start, l:end] = [l:block.output_start, l:block.output_end]
  endif
  if l:start <= 0 || l:end <= 0
    echohl ErrorMsg | echo 'sivtr: current block has no ' . a:kind . ' range' | echohl None
    return
  endif
  call cursor(l:start, 1)
  normal! V
  call cursor(l:end, 1)
endfunction

nnoremap <silent> p :qa!<CR>
nnoremap <silent> q :qa!<CR>
nnoremap <silent> <Esc> :qa!<CR>
nnoremap <silent> [[ :call <SID>SivtrJump(-1)<CR>
nnoremap <silent> ]] :call <SID>SivtrJump(1)<CR>
nnoremap <silent> myy :call <SID>SivtrCopy('block')<CR>
nnoremap <silent> myi :call <SID>SivtrCopy('input')<CR>
nnoremap <silent> myo :call <SID>SivtrCopy('output')<CR>
nnoremap <silent> myc :call <SID>SivtrCopy('command')<CR>
nnoremap <silent> mvv :call <SID>SivtrSelect('block')<CR>
nnoremap <silent> mvi :call <SID>SivtrSelect('input')<CR>
nnoremap <silent> mvo :call <SID>SivtrSelect('output')<CR>
nnoremap <silent> T :call <SID>SivtrToggleTools()<CR>
autocmd VimEnter * echo "sivtr: [[/]] jump turns, T toggles tools, myy/myi/myo/myc copy, mvv/mvi/mvo select, p returns to picker"
"#
    );
    file.write_all(script.as_bytes())
        .context("Failed to write temporary Vim config")?;
    Ok(())
}

fn build_output_preview(block: &CommandBlock) -> String {
    if block.output.trim().is_empty() {
        return "<no output>".to_string();
    }

    let mut lines: Vec<&str> = block.output.lines().take(PICK_PREVIEW_LINES).collect();
    let total_lines = block.output.lines().count();
    if total_lines > PICK_PREVIEW_LINES {
        lines.push("...");
    }
    lines.join("\n")
}

fn build_text_preview(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## "))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

fn build_text_preview_lines(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(PICK_PREVIEW_LINES)
        .collect();
    if text.lines().count() > PICK_PREVIEW_LINES {
        lines.push("...");
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::picker::{apply_range_toggle, selection_from_entries, PickEntry};
    use super::{
        agent_dialogue_vim_view, agent_session_preview, build_agent_units, build_agent_vim_view,
        build_current_agent_session_choices, build_output_preview, can_copy_agent_user_excerpt,
        can_open_dialogue_vim, extract_markdown_section, filter_lines_by_regex,
        filter_lines_by_spec, format_block, format_content_with_line_numbers, is_vim_command,
        parse_user_copy_line_spec, resolve_agent_session_selector, slice_text_by_line_spec,
        vim_single_quote, AgentBlock, AgentBlockKind, AgentHierarchyFocus, AgentProvider,
        AgentSelection, AgentSession, AgentSessionChoice, AgentSessionInfo, AgentSessionProvider,
        CommandBlock, CommandSelection, CopyMode, TextPair,
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
    fn picker_selection_keeps_disjoint_blocks() {
        let selection = selection_from_entries(&[
            PickEntry {
                recent: 1,
                preview: "latest".to_string(),
                output_preview: "out1".to_string(),
                full_preview: "out1".to_string(),
                selected: true,
            },
            PickEntry {
                recent: 2,
                preview: "second".to_string(),
                output_preview: "out2".to_string(),
                full_preview: "out2".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 4,
                preview: "fourth".to_string(),
                output_preview: "out4".to_string(),
                full_preview: "out4".to_string(),
                selected: true,
            },
        ])
        .unwrap();

        assert_eq!(selection, CommandSelection::RecentExplicit(vec![1, 4]));
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
    fn toggles_selected_range_in_picker() {
        let mut entries = vec![
            PickEntry {
                recent: 1,
                preview: "one".to_string(),
                output_preview: "out1".to_string(),
                full_preview: "out1".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 2,
                preview: "two".to_string(),
                output_preview: "out2".to_string(),
                full_preview: "out2".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 3,
                preview: "three".to_string(),
                output_preview: "out3".to_string(),
                full_preview: "out3".to_string(),
                selected: true,
            },
        ];

        apply_range_toggle(&mut entries, 0, 2);
        assert!(entries.iter().all(|entry| entry.selected));

        apply_range_toggle(&mut entries, 0, 2);
        assert!(entries.iter().all(|entry| !entry.selected));
    }

    #[test]
    fn builds_output_preview_from_first_lines() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> cargo test".to_string(),
            input_without_prompt: "cargo test".to_string(),
            output: "line1\nline2\nline3".to_string(),
            command: "cargo test".to_string(),
        };

        assert_eq!(build_output_preview(&block), "line1\nline2\nline3");
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
    fn codex_vim_view_hides_tools_by_default_and_moves_by_turn() {
        let blocks = vec![
            AgentBlock {
                kind: AgentBlockKind::User,
                timestamp: None,
                label: None,
                text: "first question".to_string(),
            },
            AgentBlock {
                kind: AgentBlockKind::ToolCall,
                timestamp: None,
                label: Some("shell".to_string()),
                text: "tool call".to_string(),
            },
            AgentBlock {
                kind: AgentBlockKind::ToolOutput,
                timestamp: None,
                label: None,
                text: "tool output".to_string(),
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
                text: "second question".to_string(),
            },
            AgentBlock {
                kind: AgentBlockKind::Assistant,
                timestamp: None,
                label: None,
                text: "second answer".to_string(),
            },
        ];

        let view = build_agent_vim_view(&blocks);

        assert!(!view.raw.contains("tool output"));
        assert_eq!(view.blocks.len(), 2);
        assert_eq!(view.blocks[0].start, 1);
        assert_eq!(view.blocks[1].start, view.blocks[0].end + 2);
        assert_eq!(view.blocks[0].input_text, "first question");
        assert_eq!(view.blocks[0].output_text, "first answer");

        let full = view.alternate.expect("tools view should exist");
        assert!(full.raw.contains("tool output"));
        assert_eq!(full.blocks.len(), 2);
        assert!(full.blocks[0].output_text.contains("tool output"));
        assert!(full.blocks[0].output_text.contains("first answer"));
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
    fn agent_vim_view_groups_multiple_assistant_messages_for_one_user() {
        let blocks = vec![
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
        ];

        let view = build_agent_vim_view(&blocks);

        assert_eq!(view.blocks.len(), 1);
        assert!(view.raw.contains("first observation"));
        assert!(view.raw.contains("final review"));
        assert!(!view.raw.contains("Cargo.toml"));

        let full = view.alternate.expect("tools view should exist");
        assert_eq!(full.blocks.len(), 1);
        assert!(full.raw.contains("Cargo.toml"));
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
    fn agent_text_picker_entries_include_all_units() {
        let units = (0..75)
            .map(|idx| TextPair {
                plain: format!("unit {idx}"),
                ansi: String::new(),
            })
            .collect::<Vec<_>>();

        let entries = super::build_text_pick_entries(&units);

        assert_eq!(entries.len(), 75);
        assert_eq!(entries[0].preview, "unit 74");
        assert_eq!(entries[74].preview, "unit 0");
    }

    #[test]
    fn can_open_dialogue_vim_accepts_sessions_when_dialogues_exist() {
        assert!(!can_open_dialogue_vim(AgentHierarchyFocus::Agents, 1));
        assert!(can_open_dialogue_vim(AgentHierarchyFocus::Sessions, 1));
        assert!(can_open_dialogue_vim(AgentHierarchyFocus::Dialogues, 1));
        assert!(can_open_dialogue_vim(AgentHierarchyFocus::Content, 1));
        assert!(!can_open_dialogue_vim(AgentHierarchyFocus::Sessions, 0));
    }

    #[test]
    fn can_copy_user_excerpt_accepts_session_focus_when_dialogues_exist() {
        assert!(!can_copy_agent_user_excerpt(AgentHierarchyFocus::Agents, 1));
        assert!(can_copy_agent_user_excerpt(
            AgentHierarchyFocus::Sessions,
            1
        ));
        assert!(can_copy_agent_user_excerpt(
            AgentHierarchyFocus::Dialogues,
            1
        ));
        assert!(can_copy_agent_user_excerpt(AgentHierarchyFocus::Content, 1));
        assert!(!can_copy_agent_user_excerpt(
            AgentHierarchyFocus::Sessions,
            0
        ));
    }

    #[test]
    fn agent_dialogue_vim_view_tracks_exact_dialogue_lines() {
        let choice = AgentSessionChoice {
            provider: AgentProvider::Codex,
            modified: SystemTime::UNIX_EPOCH,
            title: "session".to_string(),
            units: vec![
                TextPair {
                    plain: "older dialogue".to_string(),
                    ansi: "older dialogue".to_string(),
                },
                TextPair {
                    plain: "line1\nline2\nline3\nline4".to_string(),
                    ansi: "line1\nline2\nline3\nline4".to_string(),
                },
            ],
            dialogue_titles: vec!["line1".to_string(), "older dialogue".to_string()],
        };

        let view = agent_dialogue_vim_view(&choice, 0);
        assert_eq!(view.raw, "line1\nline2\nline3\nline4");
        assert_eq!(view.blocks.len(), 1);
        assert_eq!(view.blocks[0].start, 1);
        assert_eq!(view.blocks[0].end, 4);
        assert_eq!(view.blocks[0].block_text, view.raw);
        assert_eq!(view.blocks[0].input_text, view.raw);
        assert_eq!(view.blocks[0].output_text, view.raw);
        assert!(view.alternate.is_none());
    }

    #[test]
    fn format_content_with_line_numbers_adds_aligned_prefixes() {
        let text = (1..=12)
            .map(|idx| format!("line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");

        let formatted = format_content_with_line_numbers(&text);
        let lines: Vec<&str> = formatted.lines().collect();

        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0], " 1 | line 1");
        assert_eq!(lines[8], " 9 | line 9");
        assert_eq!(lines[9], "10 | line 10");
        assert_eq!(lines[11], "12 | line 12");
    }

    #[test]
    fn format_content_with_line_numbers_preserves_blank_lines() {
        let formatted = format_content_with_line_numbers("alpha\n\nomega");
        assert_eq!(formatted, "1 | alpha\n2 | \n3 | omega");
    }

    #[test]
    fn extracts_user_section_without_heading_lines() {
        let text = "## User\nline 1\nline 2\n## Assistant\nanswer";
        let actual = extract_markdown_section(text, "User", "Assistant").unwrap();
        assert_eq!(actual, "line 1\nline 2");
        assert!(!actual.contains("## User"));
        assert!(!actual.contains("## Assistant"));
    }

    #[test]
    fn slices_only_user_section_lines_for_inline_copy() {
        let text = "alpha\nbeta\ngamma\ndelta";
        assert_eq!(slice_text_by_line_spec(text, "2:3").unwrap(), "beta\ngamma");
        assert_eq!(slice_text_by_line_spec(text, "4").unwrap(), "delta");
    }

    #[test]
    fn rejects_invalid_inline_user_copy_ranges() {
        let err = parse_user_copy_line_spec("4:2", 5).unwrap_err();
        assert!(err.to_string().contains("less than or equal"));

        let err = parse_user_copy_line_spec("2:8", 4).unwrap_err();
        assert!(err.to_string().contains("exceeds 4 available line(s)"));
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
