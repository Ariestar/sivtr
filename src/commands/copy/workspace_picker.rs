use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::widgets::ListState;
use regex::Regex;

use crate::commands::command_block_selector::CommandSelection;
use crate::tui::content_view::line_count;
use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    can_open_dialogue_vim, render_workspace, selected_index, workspace_help_entries,
    workspace_hit_test, workspace_layout, WorkspaceDialogue, WorkspaceFocus, WorkspaceHelpAction,
    WorkspacePickedContent, WorkspaceSearchView, WorkspaceSession, WorkspaceSource, WorkspaceView,
};
use crate::tui::workspace_search::{
    workspace_search_has_query, workspace_search_query, workspace_search_regex,
    workspace_search_scope, WorkspaceSearchScope,
};

use super::vim::{open_vim_view, VimBlock, VimView};
use super::PICK_CANCELLED_MESSAGE;

pub(super) fn run_workspace_picker_on_terminal(
    terminal: &mut crate::tui::terminal::Tui,
    all_sessions: Vec<WorkspaceSession>,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    let mut session_state = ListState::default();
    session_state.select(Some(0));
    let mut source_state = ListState::default();
    source_state.select(Some(0));
    let mut dialogue_state = ListState::default();
    dialogue_state.select(Some(0));
    let mut help_state = ListState::default();
    help_state.select(Some(0));
    let mut focus = match initial_focus {
        WorkspaceFocus::Source => WorkspaceFocus::Source,
        WorkspaceFocus::Dialogues => WorkspaceFocus::Dialogues,
        WorkspaceFocus::Content => WorkspaceFocus::Content,
        WorkspaceFocus::Sessions => WorkspaceFocus::Sessions,
    };
    let sources = workspace_sources(&all_sessions);
    let mut selected_sources = vec![true; sources.len()];
    let mut sessions = workspace_sessions_for_sources(&all_sessions, &sources, &selected_sources);
    let mut selected_sessions = vec![false; sessions.len()];
    let mut selected_dialogues = Vec::new();
    let mut range_anchor = None;
    let mut content_scroll = 0usize;
    let mut show_help = false;
    let search_index = WorkspaceSearchIndex::new(&all_sessions);
    let mut show_search = false;
    let mut search_query = String::new();
    let mut search_output = WorkspaceSearchOutput::default();
    let mut search_cursor = 0usize;
    let mut search_dirty = true;
    let mut search_apply_pending = false;
    let mut fullscreen = None;

    loop {
        if search_dirty {
            search_output = search_index.search(&all_sessions, &search_query);
            if search_cursor >= search_output.matches.len() {
                search_cursor = 0;
            }
            search_apply_pending = true;
            search_dirty = false;
        }
        let search_has_query = workspace_search_has_query(&search_query);
        sessions = if search_has_query {
            search_output.sessions.clone()
        } else {
            workspace_sessions_for_sources(&all_sessions, &sources, &selected_sources)
        };
        if selected_sessions.len() != sessions.len() {
            selected_sessions.clear();
            selected_sessions.resize(sessions.len(), false);
        }
        let pending_match = if search_has_query && search_apply_pending {
            search_output.matches.get(search_cursor).cloned()
        } else {
            None
        };
        if let Some(matched) = &pending_match {
            selected_sessions.fill(false);
            session_state.select(
                (!sessions.is_empty())
                    .then_some(matched.session_index.min(sessions.len().saturating_sub(1))),
            );
        }
        let session_idx = selected_index(&session_state).min(sessions.len().saturating_sub(1));
        session_state.select(Some(session_idx));
        let dialogues =
            workspace_dialogues_for_sessions(&sessions, session_idx, &selected_sessions);
        let dialogue_count = dialogues.len();
        let dialogue_idx = pending_match
            .as_ref()
            .map(|matched| matched.dialogue_index)
            .unwrap_or_else(|| selected_index(&dialogue_state))
            .min(dialogue_count.saturating_sub(1));
        dialogue_state.select((dialogue_count > 0).then_some(dialogue_idx));
        if pending_match.is_some() || selected_dialogues.len() != dialogue_count {
            resize_workspace_dialogue_selection(
                dialogue_count,
                &mut selected_dialogues,
                &mut range_anchor,
            );
        }
        if let Some(matched) = pending_match {
            if let Some(dialogue) = dialogues.get(dialogue_idx) {
                content_scroll = matched
                    .line_index
                    .min(line_count(&dialogue.unit.plain).saturating_sub(1));
            } else {
                content_scroll = 0;
            }
            search_apply_pending = false;
        } else {
            content_scroll = content_scroll.min(
                workspace_content_line_count(&dialogues, &selected_dialogues, dialogue_idx)
                    .saturating_sub(1),
            );
        }

        terminal.draw(|frame| {
            render_workspace(
                frame,
                WorkspaceView {
                    sources: &sources,
                    selected_sources: &selected_sources,
                    source_state: &source_state,
                    sessions: &sessions,
                    selected_sessions: &selected_sessions,
                    session_state: &session_state,
                    dialogues: &dialogues,
                    dialogue_state: &dialogue_state,
                    selected_dialogues: &selected_dialogues,
                    range_anchor,
                    focus,
                    content_scroll,
                    show_help,
                    help_state: &help_state,
                    search: (show_search || search_has_query).then_some(WorkspaceSearchView {
                        query: &search_query,
                        scope: workspace_search_scope(&search_query),
                        result_count: sessions.len(),
                        current_match: (!search_output.matches.is_empty()).then_some(search_cursor),
                        match_count: search_output.matches.len(),
                        input_open: show_search,
                    }),
                    fullscreen,
                },
            )
        })?;

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if show_search {
                    match key.code {
                        KeyCode::Esc => {
                            show_search = false;
                            search_query.clear();
                            search_dirty = true;
                            search_apply_pending = false;
                            search_cursor = 0;
                            reset_workspace_search_state(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        KeyCode::Enter => {
                            show_search = false;
                        }
                        KeyCode::Up => {
                            move_workspace_cursor_up(
                                focus,
                                &sources,
                                &sessions,
                                dialogue_count,
                                &selected_sessions,
                                &mut source_state,
                                &mut session_state,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        KeyCode::Down => {
                            move_workspace_cursor_down(
                                focus,
                                &sources,
                                &sessions,
                                dialogue_count,
                                &selected_sessions,
                                &mut source_state,
                                &mut session_state,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        KeyCode::Backspace => {
                            search_query.pop();
                            search_dirty = true;
                            search_cursor = 0;
                            search_apply_pending = true;
                            reset_workspace_search_state(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            search_query.clear();
                            search_dirty = true;
                            search_cursor = 0;
                            search_apply_pending = true;
                            reset_workspace_search_state(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        KeyCode::Char(ch) => {
                            search_query.push(ch);
                            search_dirty = true;
                            search_cursor = 0;
                            search_apply_pending = true;
                            reset_workspace_search_state(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        _ => {}
                    }
                    continue;
                }

                if show_help {
                    match key.code {
                        KeyCode::Char('?') | KeyCode::Esc => show_help = false,
                        KeyCode::Char('q') => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                        KeyCode::Up | KeyCode::Char('k') => {
                            let next = selected_index(&help_state).saturating_sub(1);
                            help_state.select(Some(next));
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let current = selected_index(&help_state);
                            let next =
                                (current + 1).min(workspace_help_entries().len().saturating_sub(1));
                            help_state.select(Some(next));
                        }
                        KeyCode::Enter => {
                            let idx = selected_index(&help_state)
                                .min(workspace_help_entries().len().saturating_sub(1));
                            let action = workspace_help_entries()[idx].action;
                            show_help = false;
                            if let Some(picked) = apply_workspace_help_action(
                                action,
                                &mut focus,
                                &mut fullscreen,
                                &sources,
                                &mut source_state,
                                &mut selected_sources,
                                &mut selected_sessions,
                                &mut session_state,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                                &mut show_search,
                                &mut search_query,
                                &mut search_dirty,
                                &sessions,
                                &dialogues,
                                session_idx,
                                dialogue_idx,
                                dialogue_count,
                                terminal,
                            )? {
                                return Ok(picked);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char('/') => {
                        show_help = false;
                        show_search = true;
                        search_query.clear();
                        search_dirty = true;
                        search_cursor = 0;
                        search_apply_pending = true;
                        reset_workspace_search_state(
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scroll,
                        );
                    }
                    KeyCode::Char('?') => {
                        show_help = true;
                    }
                    KeyCode::Esc if search_has_query => {
                        search_query.clear();
                        search_dirty = true;
                        search_cursor = 0;
                        search_apply_pending = false;
                        reset_workspace_search_state(
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scroll,
                        );
                    }
                    KeyCode::Char('z') => {
                        fullscreen = toggle_fullscreen(fullscreen, focus);
                    }
                    KeyCode::Char('n') if search_has_query && !search_output.matches.is_empty() => {
                        search_cursor = (search_cursor + 1) % search_output.matches.len();
                        content_scroll = 0;
                        search_apply_pending = true;
                    }
                    KeyCode::Char('N') if search_has_query && !search_output.matches.is_empty() => {
                        search_cursor = search_cursor
                            .checked_sub(1)
                            .unwrap_or_else(|| search_output.matches.len().saturating_sub(1));
                        content_scroll = 0;
                        search_apply_pending = true;
                    }
                    KeyCode::Char('a') if focus == WorkspaceFocus::Source => {
                        select_sources(
                            &sources,
                            &mut selected_sources,
                            WorkspaceSourceSelection::All,
                        );
                        reset_workspace_after_source_change(
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scroll,
                        );
                    }
                    KeyCode::Char('g') if focus == WorkspaceFocus::Source => {
                        select_sources(
                            &sources,
                            &mut selected_sources,
                            WorkspaceSourceSelection::Agents,
                        );
                        reset_workspace_after_source_change(
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scroll,
                        );
                    }
                    KeyCode::Char('t') if focus == WorkspaceFocus::Source => {
                        select_sources(
                            &sources,
                            &mut selected_sources,
                            WorkspaceSourceSelection::Terminal,
                        );
                        reset_workspace_after_source_change(
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scroll,
                        );
                    }
                    KeyCode::Char('s') => {
                        set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Source);
                    }
                    KeyCode::Char(ch) if ch.is_ascii_digit() => {
                        if let Some(next_focus) =
                            WorkspaceFocus::from_number_key(ch, dialogue_count)
                        {
                            set_focus(&mut focus, &mut fullscreen, next_focus);
                        }
                    }
                    KeyCode::Char('q') => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                    KeyCode::Esc => match focus {
                        WorkspaceFocus::Source => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                        WorkspaceFocus::Sessions => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                        WorkspaceFocus::Dialogues => {
                            set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Sessions)
                        }
                        WorkspaceFocus::Content => {
                            set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Dialogues)
                        }
                    },
                    KeyCode::Left | KeyCode::Char('h') => {
                        if let Some(next_focus) = focus.previous(dialogue_count) {
                            set_focus(&mut focus, &mut fullscreen, next_focus);
                        }
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(next_focus) = focus.next(dialogue_count) {
                            set_focus(&mut focus, &mut fullscreen, next_focus);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => match focus {
                        WorkspaceFocus::Source => {
                            let next = selected_index(&source_state).saturating_sub(1);
                            source_state.select(Some(next));
                        }
                        WorkspaceFocus::Sessions => {
                            let next = selected_index(&session_state).saturating_sub(1);
                            if next != selected_index(&session_state) {
                                session_state.select(Some(next));
                                if !has_selected_sessions(&selected_sessions) {
                                    reset_workspace_dialogue_state(
                                        0,
                                        &mut dialogue_state,
                                        &mut selected_dialogues,
                                        &mut range_anchor,
                                    );
                                }
                                content_scroll = 0;
                            }
                        }
                        WorkspaceFocus::Dialogues => {
                            let next = selected_index(&dialogue_state).saturating_sub(1);
                            dialogue_state.select(Some(next));
                            content_scroll = 0;
                        }
                        WorkspaceFocus::Content => {
                            content_scroll = content_scroll.saturating_sub(1);
                        }
                    },
                    KeyCode::Down | KeyCode::Char('j') => match focus {
                        WorkspaceFocus::Source => {
                            let current = selected_index(&source_state);
                            let next = (current + 1).min(sources.len().saturating_sub(1));
                            source_state.select(Some(next));
                        }
                        WorkspaceFocus::Sessions => {
                            let current = selected_index(&session_state);
                            let next = (current + 1).min(sessions.len().saturating_sub(1));
                            if next != current {
                                session_state.select(Some(next));
                                if !has_selected_sessions(&selected_sessions) {
                                    reset_workspace_dialogue_state(
                                        0,
                                        &mut dialogue_state,
                                        &mut selected_dialogues,
                                        &mut range_anchor,
                                    );
                                }
                                content_scroll = 0;
                            }
                        }
                        WorkspaceFocus::Dialogues => {
                            let current = selected_index(&dialogue_state);
                            let next = (current + 1).min(dialogue_count.saturating_sub(1));
                            dialogue_state.select(Some(next));
                            content_scroll = 0;
                        }
                        WorkspaceFocus::Content => {
                            content_scroll = content_scroll.saturating_add(1);
                        }
                    },
                    KeyCode::PageDown | KeyCode::Char('d')
                        if focus == WorkspaceFocus::Content
                            && (key.code == KeyCode::PageDown
                                || key.modifiers.contains(KeyModifiers::CONTROL)) =>
                    {
                        content_scroll = content_scroll.saturating_add(10);
                    }
                    KeyCode::PageUp | KeyCode::Char('u')
                        if focus == WorkspaceFocus::Content
                            && (key.code == KeyCode::PageUp
                                || key.modifiers.contains(KeyModifiers::CONTROL)) =>
                    {
                        content_scroll = content_scroll.saturating_sub(10);
                    }
                    KeyCode::Char('g') if focus == WorkspaceFocus::Content => {
                        content_scroll = 0;
                    }
                    KeyCode::Char('G') if focus == WorkspaceFocus::Content => {
                        content_scroll = workspace_content_line_count(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                        )
                        .saturating_sub(1);
                    }
                    KeyCode::Char(' ') => match focus {
                        WorkspaceFocus::Source => {
                            let source_idx = selected_index(&source_state);
                            if let Some(selected) = selected_sources.get_mut(source_idx) {
                                *selected = !*selected;
                            }
                            reset_workspace_after_source_change(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scroll,
                            );
                        }
                        WorkspaceFocus::Sessions => {
                            if let Some(selected) = selected_sessions.get_mut(session_idx) {
                                *selected = !*selected;
                            }
                            reset_workspace_dialogue_state(
                                0,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                            );
                            content_scroll = 0;
                        }
                        WorkspaceFocus::Dialogues => {
                            if let Some(selected) = selected_dialogues.get_mut(dialogue_idx) {
                                *selected = !*selected;
                            }
                            range_anchor = None;
                        }
                        _ => {}
                    },
                    KeyCode::Char('v') if focus == WorkspaceFocus::Dialogues => {
                        apply_dialogue_range_selection(
                            &mut range_anchor,
                            &mut selected_dialogues,
                            dialogue_idx,
                        );
                    }
                    KeyCode::Char('a') if focus == WorkspaceFocus::Dialogues => {
                        let select_all = selected_dialogues.iter().any(|selected| !selected);
                        selected_dialogues.fill(select_all);
                        range_anchor = None;
                    }
                    KeyCode::Char('t') if can_open_dialogue_vim(focus, dialogue_count) => {
                        let view = workspace_dialogue_vim_view(&dialogues[dialogue_idx]);
                        restore_tui(terminal)?;
                        open_vim_view(&view)?;
                        *terminal = init_tui()?;
                    }
                    KeyCode::Enter => match focus {
                        WorkspaceFocus::Source => {
                            set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Sessions);
                        }
                        WorkspaceFocus::Sessions => {
                            if dialogue_count > 0 {
                                set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Dialogues);
                            }
                        }
                        WorkspaceFocus::Dialogues => {
                            return Ok(workspace_picked_content(
                                &dialogues,
                                &selected_dialogues,
                                dialogue_idx,
                            ));
                        }
                        WorkspaceFocus::Content => {
                            return Ok(workspace_picked_content(
                                &dialogues,
                                &selected_dialogues,
                                dialogue_idx,
                            ));
                        }
                    },
                    _ => {}
                }
            }
            Event::Mouse(mouse) if !show_help && !show_search => {
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                    let size = terminal.size()?;
                    let layout = workspace_layout(
                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        focus,
                        fullscreen,
                    );
                    if let Some(clicked_focus) = workspace_hit_test(layout, mouse.column, mouse.row)
                    {
                        set_focus(&mut focus, &mut fullscreen, clicked_focus);
                        match clicked_focus {
                            WorkspaceFocus::Source => {
                                if let Some(idx) = source_inline_index(
                                    layout.source,
                                    mouse.column,
                                    mouse.row,
                                    &sources,
                                ) {
                                    source_state.select(Some(idx));
                                }
                            }
                            WorkspaceFocus::Sessions => {
                                if let Some(idx) =
                                    row_list_index(layout.sessions, mouse.row, sessions.len())
                                {
                                    session_state.select(Some(idx));
                                    if !has_selected_sessions(&selected_sessions) {
                                        reset_workspace_dialogue_state(
                                            0,
                                            &mut dialogue_state,
                                            &mut selected_dialogues,
                                            &mut range_anchor,
                                        );
                                    }
                                    content_scroll = 0;
                                }
                            }
                            WorkspaceFocus::Dialogues => {
                                if let Some(idx) =
                                    row_list_index(layout.dialogues, mouse.row, dialogue_count)
                                {
                                    dialogue_state.select(Some(idx));
                                    content_scroll = 0;
                                }
                            }
                            WorkspaceFocus::Content => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn workspace_sources(sessions: &[WorkspaceSession]) -> Vec<WorkspaceSource> {
    let mut sources = Vec::new();
    for session in sessions {
        if !sources.contains(&session.source) {
            sources.push(session.source);
        }
    }
    sources
}

#[derive(Clone)]
struct WorkspaceSearchSessionEntry {
    session_index: usize,
    session_title: String,
}

#[derive(Clone)]
struct WorkspaceSearchDialogueEntry {
    session_index: usize,
    dialogue_index: usize,
    dialogue_title: String,
    content: String,
}

pub(super) struct WorkspaceSearchIndex {
    sessions: Vec<WorkspaceSearchSessionEntry>,
    dialogues: Vec<WorkspaceSearchDialogueEntry>,
}

#[derive(Default)]
pub(super) struct WorkspaceSearchOutput {
    pub(super) sessions: Vec<WorkspaceSession>,
    pub(super) matches: Vec<WorkspaceSearchMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceSearchMatch {
    pub(super) session_index: usize,
    pub(super) dialogue_index: usize,
    pub(super) line_index: usize,
}

impl WorkspaceSearchIndex {
    pub(super) fn new(sessions: &[WorkspaceSession]) -> Self {
        let mut session_entries = Vec::with_capacity(sessions.len());
        let dialogue_count = sessions
            .iter()
            .map(|session| session.dialogue_titles.len())
            .sum();
        let mut dialogue_entries = Vec::with_capacity(dialogue_count);

        for (session_index, session) in sessions.iter().enumerate() {
            session_entries.push(WorkspaceSearchSessionEntry {
                session_index,
                session_title: session.title.clone(),
            });

            for (dialogue_index, (dialogue_title, unit)) in session
                .dialogue_titles
                .iter()
                .zip(session.units.iter())
                .enumerate()
            {
                dialogue_entries.push(WorkspaceSearchDialogueEntry {
                    session_index,
                    dialogue_index,
                    dialogue_title: dialogue_title.clone(),
                    content: unit.plain.clone(),
                });
            }
        }

        Self {
            sessions: session_entries,
            dialogues: dialogue_entries,
        }
    }

    pub(super) fn search(
        &self,
        all_sessions: &[WorkspaceSession],
        query: &str,
    ) -> WorkspaceSearchOutput {
        let (scope, term) = workspace_search_query(query);
        let Some(regex) = workspace_search_regex(term) else {
            return WorkspaceSearchOutput::default();
        };
        match scope {
            WorkspaceSearchScope::Session => {
                let mut sessions = Vec::new();
                let mut matches = Vec::new();
                for entry in self
                    .sessions
                    .iter()
                    .filter(|entry| regex.is_match(&entry.session_title))
                {
                    let filtered_session_index = sessions.len();
                    if let Some(session) = all_sessions.get(entry.session_index).cloned() {
                        sessions.push(session);
                        matches.push(WorkspaceSearchMatch {
                            session_index: filtered_session_index,
                            dialogue_index: 0,
                            line_index: 0,
                        });
                    }
                }
                WorkspaceSearchOutput { sessions, matches }
            }
            WorkspaceSearchScope::Dialogue => self.search_dialogue_titles(all_sessions, &regex),
            WorkspaceSearchScope::Content => self.search_dialogue_content(all_sessions, &regex),
        }
    }

    fn search_dialogue_titles(
        &self,
        all_sessions: &[WorkspaceSession],
        regex: &Regex,
    ) -> WorkspaceSearchOutput {
        let mut grouped: Vec<(usize, Vec<usize>)> = Vec::new();
        for entry in self
            .dialogues
            .iter()
            .filter(|entry| regex.is_match(&entry.dialogue_title))
        {
            if let Some((_, dialogue_indices)) = grouped
                .iter_mut()
                .find(|(session_index, _)| *session_index == entry.session_index)
            {
                dialogue_indices.push(entry.dialogue_index);
            } else {
                grouped.push((entry.session_index, vec![entry.dialogue_index]));
            }
        }

        let sessions = grouped
            .into_iter()
            .filter_map(|(session_index, dialogue_indices)| {
                let session = all_sessions.get(session_index)?;
                Some(filter_workspace_session_dialogues(
                    session,
                    &dialogue_indices,
                ))
            })
            .collect::<Vec<_>>();
        let matches = sessions
            .iter()
            .enumerate()
            .flat_map(|(session_index, session)| {
                session
                    .dialogue_titles
                    .iter()
                    .enumerate()
                    .filter(|(_, title)| regex.is_match(title))
                    .map(move |(dialogue_index, _)| WorkspaceSearchMatch {
                        session_index,
                        dialogue_index,
                        line_index: 0,
                    })
            })
            .collect();
        WorkspaceSearchOutput { sessions, matches }
    }

    fn search_dialogue_content(
        &self,
        all_sessions: &[WorkspaceSession],
        regex: &Regex,
    ) -> WorkspaceSearchOutput {
        let mut grouped: Vec<(usize, Vec<usize>)> = Vec::new();
        for entry in self
            .dialogues
            .iter()
            .filter(|entry| regex.is_match(&entry.content))
        {
            if let Some((_, dialogue_indices)) = grouped
                .iter_mut()
                .find(|(session_index, _)| *session_index == entry.session_index)
            {
                dialogue_indices.push(entry.dialogue_index);
            } else {
                grouped.push((entry.session_index, vec![entry.dialogue_index]));
            }
        }

        let sessions = grouped
            .into_iter()
            .filter_map(|(session_index, dialogue_indices)| {
                let session = all_sessions.get(session_index)?;
                Some(filter_workspace_session_dialogues(
                    session,
                    &dialogue_indices,
                ))
            })
            .collect::<Vec<_>>();
        let matches = sessions
            .iter()
            .enumerate()
            .flat_map(|(session_index, session)| {
                session
                    .units
                    .iter()
                    .enumerate()
                    .flat_map(move |(dialogue_index, unit)| {
                        unit.plain
                            .lines()
                            .enumerate()
                            .filter(|(_, line)| regex.is_match(line))
                            .map(move |(line_index, _)| WorkspaceSearchMatch {
                                session_index,
                                dialogue_index,
                                line_index,
                            })
                    })
            })
            .collect();
        WorkspaceSearchOutput { sessions, matches }
    }
}

fn filter_workspace_session_dialogues(
    session: &WorkspaceSession,
    dialogue_indices: &[usize],
) -> WorkspaceSession {
    let mut filtered = session.clone();
    filtered.dialogue_titles = dialogue_indices
        .iter()
        .filter_map(|idx| session.dialogue_titles.get(*idx).cloned())
        .collect();
    filtered.units = dialogue_indices
        .iter()
        .filter_map(|idx| session.units.get(*idx).cloned())
        .collect();
    filtered
}

#[allow(clippy::too_many_arguments)]
fn apply_workspace_help_action(
    action: WorkspaceHelpAction,
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    sources: &[WorkspaceSource],
    source_state: &mut ListState,
    selected_sources: &mut Vec<bool>,
    selected_sessions: &mut Vec<bool>,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scroll: &mut usize,
    show_search: &mut bool,
    search_query: &mut String,
    search_dirty: &mut bool,
    sessions: &[WorkspaceSession],
    dialogues: &[WorkspaceDialogue],
    session_idx: usize,
    dialogue_idx: usize,
    dialogue_count: usize,
    terminal: &mut crate::tui::terminal::Tui,
) -> Result<Option<WorkspacePickedContent>> {
    match action {
        WorkspaceHelpAction::FocusSource => set_focus(focus, fullscreen, WorkspaceFocus::Source),
        WorkspaceHelpAction::FocusSessions => {
            set_focus(focus, fullscreen, WorkspaceFocus::Sessions)
        }
        WorkspaceHelpAction::FocusDialogues if dialogue_count > 0 => {
            set_focus(focus, fullscreen, WorkspaceFocus::Dialogues)
        }
        WorkspaceHelpAction::FocusContent if dialogue_count > 0 => {
            set_focus(focus, fullscreen, WorkspaceFocus::Content)
        }
        WorkspaceHelpAction::MoveUp => match *focus {
            WorkspaceFocus::Source => {
                let next = selected_index(source_state).saturating_sub(1);
                source_state.select(Some(next));
            }
            WorkspaceFocus::Sessions => {
                let next = selected_index(session_state).saturating_sub(1);
                if next != selected_index(session_state) {
                    session_state.select(Some(next));
                    if !has_selected_sessions(selected_sessions) {
                        reset_workspace_dialogue_state(
                            0,
                            dialogue_state,
                            selected_dialogues,
                            range_anchor,
                        );
                    }
                    *content_scroll = 0;
                }
            }
            WorkspaceFocus::Dialogues => {
                dialogue_state.select(Some(selected_index(dialogue_state).saturating_sub(1)));
                *content_scroll = 0;
            }
            WorkspaceFocus::Content => *content_scroll = (*content_scroll).saturating_sub(1),
        },
        WorkspaceHelpAction::MoveDown => match *focus {
            WorkspaceFocus::Source => {
                let current = selected_index(source_state);
                let next = (current + 1).min(sources.len().saturating_sub(1));
                source_state.select(Some(next));
            }
            WorkspaceFocus::Sessions => {
                let current = selected_index(session_state);
                let next = (current + 1).min(sessions.len().saturating_sub(1));
                if next != current {
                    session_state.select(Some(next));
                    if !has_selected_sessions(selected_sessions) {
                        reset_workspace_dialogue_state(
                            0,
                            dialogue_state,
                            selected_dialogues,
                            range_anchor,
                        );
                    }
                    *content_scroll = 0;
                }
            }
            WorkspaceFocus::Dialogues => {
                let current = selected_index(dialogue_state);
                dialogue_state.select(Some((current + 1).min(dialogue_count.saturating_sub(1))));
                *content_scroll = 0;
            }
            WorkspaceFocus::Content => *content_scroll = (*content_scroll).saturating_add(1),
        },
        WorkspaceHelpAction::PreviousPane => {
            if let Some(next_focus) = focus.previous(dialogue_count) {
                set_focus(focus, fullscreen, next_focus);
            }
        }
        WorkspaceHelpAction::NextPane => {
            if let Some(next_focus) = focus.next(dialogue_count) {
                set_focus(focus, fullscreen, next_focus);
            }
        }
        WorkspaceHelpAction::ToggleSelection => match *focus {
            WorkspaceFocus::Source => {
                let source_idx = selected_index(source_state);
                if let Some(selected) = selected_sources.get_mut(source_idx) {
                    *selected = !*selected;
                }
                reset_workspace_after_source_change(
                    session_state,
                    selected_sessions,
                    dialogue_state,
                    selected_dialogues,
                    range_anchor,
                    content_scroll,
                );
            }
            WorkspaceFocus::Sessions => {
                if let Some(selected) = selected_sessions.get_mut(session_idx) {
                    *selected = !*selected;
                }
                reset_workspace_dialogue_state(0, dialogue_state, selected_dialogues, range_anchor);
                *content_scroll = 0;
            }
            WorkspaceFocus::Dialogues => {
                if let Some(selected) = selected_dialogues.get_mut(dialogue_idx) {
                    *selected = !*selected;
                }
                *range_anchor = None;
            }
            _ => {}
        },
        WorkspaceHelpAction::SelectAllSources => {
            select_sources(sources, selected_sources, WorkspaceSourceSelection::All);
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scroll,
            );
        }
        WorkspaceHelpAction::SelectAgentSources => {
            select_sources(sources, selected_sources, WorkspaceSourceSelection::Agents);
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scroll,
            );
        }
        WorkspaceHelpAction::SelectTerminalSource => {
            select_sources(
                sources,
                selected_sources,
                WorkspaceSourceSelection::Terminal,
            );
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scroll,
            );
        }
        WorkspaceHelpAction::RangeSelect if *focus == WorkspaceFocus::Dialogues => {
            apply_dialogue_range_selection(range_anchor, selected_dialogues, dialogue_idx);
        }
        WorkspaceHelpAction::ToggleAllDialogues if *focus == WorkspaceFocus::Dialogues => {
            let select_all = selected_dialogues.iter().any(|selected| !selected);
            selected_dialogues.fill(select_all);
            *range_anchor = None;
        }
        WorkspaceHelpAction::OpenVim if can_open_dialogue_vim(*focus, dialogue_count) => {
            let view = workspace_dialogue_vim_view(&dialogues[dialogue_idx]);
            restore_tui(terminal)?;
            open_vim_view(&view)?;
            *terminal = init_tui()?;
        }
        WorkspaceHelpAction::ScrollDown if *focus == WorkspaceFocus::Content => {
            *content_scroll = (*content_scroll).saturating_add(10);
        }
        WorkspaceHelpAction::ScrollUp if *focus == WorkspaceFocus::Content => {
            *content_scroll = (*content_scroll).saturating_sub(10);
        }
        WorkspaceHelpAction::Copy => match *focus {
            WorkspaceFocus::Source => set_focus(focus, fullscreen, WorkspaceFocus::Sessions),
            WorkspaceFocus::Sessions if dialogue_count > 0 => {
                set_focus(focus, fullscreen, WorkspaceFocus::Dialogues)
            }
            WorkspaceFocus::Dialogues | WorkspaceFocus::Content => {
                return Ok(Some(workspace_picked_content(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
                )));
            }
            WorkspaceFocus::Sessions => {}
        },
        WorkspaceHelpAction::ToggleFullscreen => {
            *fullscreen = toggle_fullscreen(*fullscreen, *focus);
        }
        WorkspaceHelpAction::CloseHelp => {}
        WorkspaceHelpAction::OpenSearch => {
            *show_search = true;
            search_query.clear();
            *search_dirty = true;
            reset_workspace_search_state(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scroll,
            );
        }
        WorkspaceHelpAction::Cancel => anyhow::bail!(PICK_CANCELLED_MESSAGE),
        _ => {}
    }

    Ok(None)
}

fn toggle_fullscreen(
    fullscreen: Option<WorkspaceFocus>,
    focus: WorkspaceFocus,
) -> Option<WorkspaceFocus> {
    if fullscreen == Some(focus) {
        None
    } else {
        Some(focus)
    }
}

fn set_focus(
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    next: WorkspaceFocus,
) {
    *focus = next;
    if fullscreen.is_some() {
        *fullscreen = Some(next);
    }
}

pub(super) fn workspace_picked_content(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
) -> WorkspacePickedContent {
    let selected_indices = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    let picked_indices = if selected_indices.is_empty() {
        vec![dialogue_idx]
    } else {
        selected_indices
    };
    let source_idx = picked_indices[0];
    let units = picked_indices
        .into_iter()
        .filter_map(|idx| dialogues.get(idx))
        .map(|dialogue| dialogue.unit.clone())
        .collect::<Vec<_>>();
    let selection = CommandSelection::RecentExplicit((1..=units.len()).collect());
    WorkspacePickedContent {
        source: dialogues[source_idx].source,
        units,
        selection,
    }
}

fn workspace_content_line_count(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> usize {
    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return dialogues
            .get(highlighted_idx)
            .map(|dialogue| line_count(&dialogue.unit.plain))
            .unwrap_or(1);
    }

    let text = selected
        .into_iter()
        .filter_map(|dialogue_idx| dialogues.get(dialogue_idx))
        .map(|dialogue| dialogue.unit.plain.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    line_count(&text)
}

fn apply_dialogue_range_selection(
    range_anchor: &mut Option<usize>,
    selected_dialogues: &mut [bool],
    dialogue_idx: usize,
) {
    if let Some(anchor) = range_anchor.take() {
        let start = anchor.min(dialogue_idx);
        let end = anchor.max(dialogue_idx);
        let select = selected_dialogues
            .get(start..=end)
            .map(|range| range.iter().any(|selected| !selected))
            .unwrap_or(true);
        for idx in start..=end {
            if let Some(selected) = selected_dialogues.get_mut(idx) {
                *selected = select;
            }
        }
    } else {
        *range_anchor = Some(dialogue_idx);
    }
}

fn workspace_sessions_for_sources(
    all_sessions: &[WorkspaceSession],
    sources: &[WorkspaceSource],
    selected_sources: &[bool],
) -> Vec<WorkspaceSession> {
    let mut sessions = all_sessions
        .iter()
        .filter(|session| {
            sources
                .iter()
                .position(|source| *source == session.source)
                .and_then(|idx| selected_sources.get(idx))
                .copied()
                .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
    sessions
}

#[derive(Clone, Copy)]
enum WorkspaceSourceSelection {
    All,
    Agents,
    Terminal,
}

fn select_sources(
    sources: &[WorkspaceSource],
    selected_sources: &mut [bool],
    selection: WorkspaceSourceSelection,
) {
    for (idx, source) in sources.iter().enumerate() {
        if let Some(selected) = selected_sources.get_mut(idx) {
            *selected = match selection {
                WorkspaceSourceSelection::All => true,
                WorkspaceSourceSelection::Agents => source.is_agent(),
                WorkspaceSourceSelection::Terminal => source.is_terminal(),
            };
        }
    }
}

fn has_selected_sessions(selected_sessions: &[bool]) -> bool {
    selected_sessions.iter().any(|selected| *selected)
}

pub(super) fn workspace_dialogues_for_sessions(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
) -> Vec<WorkspaceDialogue> {
    let selected_indices = selected_sessions
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    let session_indices = if selected_indices.is_empty() {
        vec![session_idx]
    } else {
        selected_indices
    };

    session_indices
        .into_iter()
        .filter_map(|idx| sessions.get(idx))
        .flat_map(|session| {
            session
                .dialogue_titles
                .iter()
                .cloned()
                .zip(session.units.iter().cloned())
                .map(move |(title, unit)| WorkspaceDialogue {
                    source: session.source,
                    title,
                    unit,
                })
        })
        .collect()
}

fn reset_workspace_after_source_change(
    session_state: &mut ListState,
    selected_sessions: &mut Vec<bool>,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scroll: &mut usize,
) {
    session_state.select(Some(0));
    selected_sessions.clear();
    dialogue_state.select(Some(0));
    selected_dialogues.clear();
    *range_anchor = None;
    *content_scroll = 0;
}

fn reset_workspace_search_state(
    session_state: &mut ListState,
    selected_sessions: &mut Vec<bool>,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scroll: &mut usize,
) {
    session_state.select(Some(0));
    selected_sessions.clear();
    dialogue_state.select(Some(0));
    selected_dialogues.clear();
    *range_anchor = None;
    *content_scroll = 0;
}

fn resize_workspace_dialogue_selection(
    dialogue_count: usize,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
) {
    selected_dialogues.clear();
    selected_dialogues.resize(dialogue_count, false);
    *range_anchor = None;
}

#[allow(clippy::too_many_arguments)]
fn move_workspace_cursor_up(
    focus: WorkspaceFocus,
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    _dialogue_count: usize,
    selected_sessions: &[bool],
    source_state: &mut ListState,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scroll: &mut usize,
) {
    match focus {
        WorkspaceFocus::Source => {
            let next = selected_index(source_state).saturating_sub(1);
            source_state.select((!sources.is_empty()).then_some(next));
        }
        WorkspaceFocus::Sessions => {
            let next = selected_index(session_state).saturating_sub(1);
            if next != selected_index(session_state) {
                session_state.select(Some(next));
                if !has_selected_sessions(selected_sessions) {
                    reset_workspace_dialogue_state(
                        0,
                        dialogue_state,
                        selected_dialogues,
                        range_anchor,
                    );
                }
                *content_scroll = 0;
            }
        }
        WorkspaceFocus::Dialogues => {
            let next = selected_index(dialogue_state).saturating_sub(1);
            dialogue_state.select((!sessions.is_empty()).then_some(next));
            *content_scroll = 0;
        }
        WorkspaceFocus::Content => {
            *content_scroll = content_scroll.saturating_sub(1);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn move_workspace_cursor_down(
    focus: WorkspaceFocus,
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    dialogue_count: usize,
    selected_sessions: &[bool],
    source_state: &mut ListState,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scroll: &mut usize,
) {
    match focus {
        WorkspaceFocus::Source => {
            let current = selected_index(source_state);
            let next = (current + 1).min(sources.len().saturating_sub(1));
            source_state.select(Some(next));
        }
        WorkspaceFocus::Sessions => {
            let current = selected_index(session_state);
            let next = (current + 1).min(sessions.len().saturating_sub(1));
            if next != current {
                session_state.select(Some(next));
                if !has_selected_sessions(selected_sessions) {
                    reset_workspace_dialogue_state(
                        0,
                        dialogue_state,
                        selected_dialogues,
                        range_anchor,
                    );
                }
                *content_scroll = 0;
            }
        }
        WorkspaceFocus::Dialogues => {
            let current = selected_index(dialogue_state);
            let next = (current + 1).min(dialogue_count.saturating_sub(1));
            dialogue_state.select((dialogue_count > 0).then_some(next));
            *content_scroll = 0;
        }
        WorkspaceFocus::Content => {
            *content_scroll = content_scroll.saturating_add(1);
        }
    }
}

fn row_list_index(area: ratatui::layout::Rect, row: u16, len: usize) -> Option<usize> {
    let row = row.checked_sub(area.y.saturating_add(1))? as usize;
    (row < len).then_some(row)
}

fn source_inline_index(
    area: ratatui::layout::Rect,
    column: u16,
    row: u16,
    sources: &[WorkspaceSource],
) -> Option<usize> {
    if row != area.y.saturating_add(1)
        || column <= area.x
        || column >= area.x.saturating_add(area.width)
    {
        return None;
    }

    let mut cursor = area.x.saturating_add(1);
    for (idx, source) in sources.iter().enumerate() {
        if idx > 0 {
            cursor = cursor.saturating_add(2);
        }
        let width = source.label().len() as u16 + 4;
        if column >= cursor && column < cursor.saturating_add(width) {
            return Some(idx);
        }
        cursor = cursor.saturating_add(width);
    }

    None
}

fn reset_workspace_dialogue_state(
    dialogue_count: usize,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
) {
    dialogue_state.select((dialogue_count > 0).then_some(0));
    selected_dialogues.clear();
    selected_dialogues.resize(dialogue_count, false);
    *range_anchor = None;
}

pub(super) fn workspace_dialogue_vim_view(dialogue: &WorkspaceDialogue) -> VimView {
    dialogue_text_vim_view(dialogue.unit.plain.clone())
}

fn dialogue_text_vim_view(text: String) -> VimView {
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
        raw: text,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        workspace_dialogue_vim_view, workspace_dialogues_for_sessions, workspace_picked_content,
        WorkspaceSearchIndex, WorkspaceSearchMatch,
    };
    use crate::commands::command_block_selector::CommandSelection;
    use crate::tui::workspace::{TextPair, WorkspaceDialogue, WorkspaceSession, WorkspaceSource};
    use crate::tui::workspace_search::{
        workspace_search_query, workspace_search_regex, WorkspaceSearchScope,
    };
    use sivtr_core::ai::AgentProvider;
    use std::time::SystemTime;

    #[test]
    fn workspace_dialogues_follow_current_session_without_session_selection() {
        let sessions = vec![
            workspace_test_session("new", WorkspaceSource::Agent(AgentProvider::Codex), &["n1"]),
            workspace_test_session(
                "old",
                WorkspaceSource::Agent(AgentProvider::Claude),
                &["o1"],
            ),
        ];

        let dialogues = workspace_dialogues_for_sessions(&sessions, 1, &[false, false]);

        assert_eq!(dialogues.len(), 1);
        assert_eq!(dialogues[0].title, "o1");
        assert_eq!(dialogues[0].unit.plain, "old:o1");
    }

    #[test]
    fn workspace_dialogues_aggregate_selected_sessions() {
        let sessions = vec![
            workspace_test_session(
                "codex session",
                WorkspaceSource::Agent(AgentProvider::Codex),
                &["c1", "c2"],
            ),
            workspace_test_session(
                "claude session",
                WorkspaceSource::Agent(AgentProvider::Claude),
                &["a1"],
            ),
        ];

        let dialogues = workspace_dialogues_for_sessions(&sessions, 0, &[true, true]);

        assert_eq!(dialogues.len(), 3);
        assert_eq!(dialogues[0].title, "c1");
        assert_eq!(dialogues[1].title, "c2");
        assert_eq!(dialogues[2].title, "a1");
        assert_eq!(
            dialogues
                .iter()
                .map(|dialogue| dialogue.unit.plain.as_str())
                .collect::<Vec<_>>(),
            vec!["codex session:c1", "codex session:c2", "claude session:a1"]
        );
    }

    #[test]
    fn workspace_search_defaults_to_dialogue_content() {
        let sessions = vec![
            workspace_test_session(
                "alpha session",
                WorkspaceSource::Agent(AgentProvider::Codex),
                &["camera"],
            ),
            workspace_test_session(
                "target session",
                WorkspaceSource::Agent(AgentProvider::Claude),
                &["lighting"],
            ),
        ];
        let index = WorkspaceSearchIndex::new(&sessions);

        let output = index.search(&sessions, "target session:lighting");

        assert_eq!(
            workspace_search_query("target session:lighting").0,
            WorkspaceSearchScope::Content
        );
        assert_eq!(output.sessions.len(), 1);
        assert_eq!(
            output.sessions[0].source,
            WorkspaceSource::Agent(AgentProvider::Claude)
        );
        assert_eq!(output.sessions[0].title, "target session");
        assert_eq!(output.sessions[0].dialogue_titles, vec!["lighting"]);
        assert_eq!(output.sessions[0].units[0].plain, "target session:lighting");
        assert_eq!(output.matches.len(), 1);
    }

    #[test]
    fn workspace_search_prefixes_select_session_or_dialogue_scope() {
        let sessions = vec![workspace_test_session(
            "photo critique",
            WorkspaceSource::Agent(AgentProvider::Codex),
            &["lighting notes"],
        )];
        let index = WorkspaceSearchIndex::new(&sessions);

        let session_results = index.search(&sessions, ">photo");
        let dialogue_results = index.search(&sessions, "#lighting");
        let content_results = index.search(&sessions, ">lighting");

        assert_eq!(
            workspace_search_query(">photo").0,
            WorkspaceSearchScope::Session
        );
        assert_eq!(
            workspace_search_query("#lighting").0,
            WorkspaceSearchScope::Dialogue
        );
        assert_eq!(session_results.sessions.len(), 1);
        assert_eq!(dialogue_results.sessions.len(), 1);
        assert_eq!(
            dialogue_results.sessions[0].dialogue_titles,
            vec!["lighting notes"]
        );
        assert!(content_results.sessions.is_empty());
    }

    #[test]
    fn workspace_search_uses_case_insensitive_regex() {
        let sessions = vec![workspace_test_session(
            "Photo critique",
            WorkspaceSource::Agent(AgentProvider::Codex),
            &["LIGHTING notes"],
        )];
        let index = WorkspaceSearchIndex::new(&sessions);

        let session_results = index.search(&sessions, ">photo\\s+critique");
        let dialogue_results = index.search(&sessions, "#lighting\\s+notes");
        let content_results = index.search(&sessions, "photo critique:lighting\\s+notes");

        assert_eq!(session_results.sessions.len(), 1);
        assert_eq!(dialogue_results.sessions.len(), 1);
        assert_eq!(content_results.sessions.len(), 1);
    }

    #[test]
    fn workspace_search_invalid_regex_has_no_fallback_matches() {
        let sessions = vec![workspace_test_session(
            "photo critique",
            WorkspaceSource::Agent(AgentProvider::Codex),
            &["lighting notes"],
        )];
        let index = WorkspaceSearchIndex::new(&sessions);

        assert!(workspace_search_regex("(").is_none());
        assert!(index.search(&sessions, "(").sessions.is_empty());
        assert!(index.search(&sessions, ">photo(").sessions.is_empty());
        assert!(index.search(&sessions, "#lighting(").sessions.is_empty());
    }

    #[test]
    fn workspace_search_filters_dialogues_inside_matching_sessions() {
        let sessions = vec![
            workspace_test_session(
                "codex session",
                WorkspaceSource::Agent(AgentProvider::Codex),
                &["needle first", "miss"],
            ),
            workspace_test_session(
                "claude session",
                WorkspaceSource::Agent(AgentProvider::Claude),
                &["a1", "needle dialogue"],
            ),
        ];
        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "#needle");

        assert_eq!(output.sessions.len(), 2);
        assert_eq!(output.sessions[0].title, "codex session");
        assert_eq!(output.sessions[0].dialogue_titles, vec!["needle first"]);
        assert_eq!(output.sessions[1].title, "claude session");
        assert_eq!(output.sessions[1].dialogue_titles, vec!["needle dialogue"]);
        assert_eq!(output.matches.len(), 2);
    }

    #[test]
    fn workspace_search_tracks_match_position_for_navigation() {
        let sessions = vec![WorkspaceSession {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            modified: SystemTime::UNIX_EPOCH,
            title: "session".to_string(),
            units: vec![TextPair {
                plain: "first\nneedle one\nmiddle\nneedle two".to_string(),
                ansi: String::new(),
            }],
            dialogue_titles: vec!["dialogue".to_string()],
        }];

        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "needle");

        assert_eq!(
            output.matches,
            vec![
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    line_index: 1
                },
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    line_index: 3
                }
            ]
        );
    }

    #[test]
    fn workspace_picked_content_uses_selected_dialogues_only() {
        let dialogues = vec![
            workspace_test_dialogue("d1", "text 1"),
            workspace_test_dialogue("d2", "text 2"),
            workspace_test_dialogue("d3", "text 3"),
        ];

        let picked = workspace_picked_content(&dialogues, &[false, true, true], 0);

        assert_eq!(
            picked
                .units
                .iter()
                .map(|unit| unit.plain.as_str())
                .collect::<Vec<_>>(),
            vec!["text 2", "text 3"]
        );
        assert_eq!(
            picked.selection,
            CommandSelection::RecentExplicit(vec![1, 2])
        );
    }

    #[test]
    fn workspace_picked_content_falls_back_to_highlighted_dialogue() {
        let dialogues = vec![
            workspace_test_dialogue("d1", "text 1"),
            workspace_test_dialogue("d2", "text 2"),
        ];

        let picked = workspace_picked_content(&dialogues, &[false, false], 1);

        assert_eq!(picked.units.len(), 1);
        assert_eq!(picked.units[0].plain, "text 2");
        assert_eq!(picked.selection, CommandSelection::RecentExplicit(vec![1]));
    }

    #[test]
    fn workspace_dialogue_vim_view_tracks_exact_dialogue_lines() {
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            title: "line1".to_string(),
            unit: TextPair {
                plain: "line1\nline2\nline3\nline4".to_string(),
                ansi: "line1\nline2\nline3\nline4".to_string(),
            },
        };

        let view = workspace_dialogue_vim_view(&dialogue);
        assert_eq!(view.raw, "line1\nline2\nline3\nline4");
        assert_eq!(view.blocks.len(), 1);
        assert_eq!(view.blocks[0].start, 1);
        assert_eq!(view.blocks[0].end, 4);
        assert_eq!(view.blocks[0].block_text, view.raw);
        assert_eq!(view.blocks[0].input_text, view.raw);
        assert_eq!(view.blocks[0].output_text, view.raw);
    }

    fn workspace_test_session(
        title: &str,
        source: WorkspaceSource,
        dialogue_titles: &[&str],
    ) -> WorkspaceSession {
        WorkspaceSession {
            source,
            modified: SystemTime::UNIX_EPOCH,
            title: title.to_string(),
            units: dialogue_titles
                .iter()
                .map(|dialogue_title| TextPair {
                    plain: format!("{title}:{dialogue_title}"),
                    ansi: format!("{title}:{dialogue_title}"),
                })
                .collect(),
            dialogue_titles: dialogue_titles
                .iter()
                .map(|dialogue_title| dialogue_title.to_string())
                .collect(),
        }
    }

    fn workspace_test_dialogue(title: &str, plain: &str) -> WorkspaceDialogue {
        WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            title: title.to_string(),
            unit: TextPair {
                plain: plain.to_string(),
                ansi: plain.to_string(),
            },
        }
    }
}
