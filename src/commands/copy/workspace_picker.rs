use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::widgets::ListState;
use std::process::Command;

use crate::commands::command_block_selector::CommandSelection;
use crate::tui::content_view::{
    clamp_content_position, content_link_at, content_position_in_text_row, content_text_area,
    content_view_line_count, line_count, selected_content_text, ContentPosition, ContentSelection,
    ContentSelectionKind, ContentViewMode,
};
use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    can_open_dialogue_vim, render_workspace, selected_index, workspace_help_entries,
    workspace_hit_test, workspace_layout, TextPair, WorkspaceDialogue, WorkspaceFocus,
    WorkspaceHelpAction, WorkspacePickedContent, WorkspaceSearchView, WorkspaceSession,
    WorkspaceSource, WorkspaceView,
};
use crate::tui::workspace_search::{
    workspace_search_has_query, workspace_search_scope, WorkspaceSearchIndex, WorkspaceSearchOutput,
};

use super::vim::{open_vim_view, VimBlock, VimView};
use super::{
    filter_lines_by_spec, load_workspace_session_at, load_workspace_sessions_for_indices,
    PICK_CANCELLED_MESSAGE,
};

const MOUSE_SCROLL_LINES: usize = 3;

#[derive(Clone, Copy)]
struct VisualSelectMode {
    selection: ContentSelection,
    dragging: bool,
}

struct VisualContentContext<'a> {
    area: ratatui::layout::Rect,
    text: &'a str,
    mode: ContentViewMode,
    scroll: usize,
}

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
    let mut focus = initial_focus;
    let mut all_sessions = all_sessions;
    let sources = workspace_sources(&all_sessions);
    let mut selected_sources = vec![true; sources.len()];
    let mut sessions = workspace_sessions_for_sources(&all_sessions, &sources, &selected_sources);
    let mut selected_sessions = vec![false; sessions.len()];
    let mut selected_dialogues = Vec::new();
    let mut range_anchor = None;
    let mut content_scroll = 0usize;
    let mut content_mode = ContentViewMode::Reading;
    let mut show_help = false;
    let mut show_search = false;
    let mut search_query = String::new();
    let mut search_output = WorkspaceSearchOutput::default();
    let mut search_cursor = 0usize;
    let mut search_dirty = true;
    let mut search_apply_pending = false;
    let mut line_filter_input_open = false;
    let mut line_filter = String::new();
    let mut line_filter_error: Option<String> = None;
    let mut fullscreen = None;
    let mut visual_select_mode = None;
    let mut search_index = WorkspaceSearchIndex::new(&all_sessions);

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
        if !search_has_query {
            let session_idx = selected_index(&session_state).min(sessions.len().saturating_sub(1));
            load_workspace_dialogue_sessions_with_cache(
                &mut all_sessions,
                &mut sessions,
                session_idx,
                &selected_sessions,
            )?;
        }
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
            let size = terminal.size()?;
            let layout = workspace_layout(
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                focus,
                fullscreen,
            );
            content_scroll = content_scroll.min(
                workspace_content_line_count(
                    &dialogues,
                    &selected_dialogues,
                    dialogue_idx,
                    layout.content,
                    content_mode,
                )
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
                    content_mode,
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
                    line_filter_input_open,
                    line_filter: (!line_filter.is_empty()).then_some(line_filter.as_str()),
                    line_filter_error: line_filter_error.as_deref(),
                    fullscreen,
                    content_selection: visual_select_mode
                        .map(|mode: VisualSelectMode| mode.selection),
                },
            )
        })?;
        if visual_select_mode.is_some() {
            terminal.show_cursor()?;
        }

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if let Some(mode) = visual_select_mode.as_mut() {
                    let size = terminal.size()?;
                    let layout = workspace_layout(
                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        focus,
                        fullscreen,
                    );
                    let text =
                        workspace_display_text(&dialogues, &selected_dialogues, dialogue_idx);
                    if let Some(picked) = handle_visual_select_key(
                        key.code,
                        key.modifiers,
                        mode,
                        layout.content,
                        &text,
                        content_mode,
                        &mut content_scroll,
                        &dialogues,
                        &selected_dialogues,
                        dialogue_idx,
                    )? {
                        return Ok(picked);
                    }
                    if matches!(key.code, KeyCode::Esc | KeyCode::Char('v')) {
                        visual_select_mode = None;
                        terminal.hide_cursor()?;
                    }
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
                            if matches!(
                                action,
                                WorkspaceHelpAction::OpenVim
                                    | WorkspaceHelpAction::Copy
                                    | WorkspaceHelpAction::CopyInput
                                    | WorkspaceHelpAction::CopyOutput
                                    | WorkspaceHelpAction::CopyBlock
                                    | WorkspaceHelpAction::CopyCommand
                            ) {
                                load_workspace_dialogue_sessions_with_cache(
                                    &mut all_sessions,
                                    &mut sessions,
                                    session_idx,
                                    &selected_sessions,
                                )?;
                            }
                            let dialogues = workspace_dialogues_for_sessions(
                                &sessions,
                                session_idx,
                                &selected_sessions,
                            );
                            let dialogue_count = dialogues.len();
                            let dialogue_idx = dialogue_idx.min(dialogue_count.saturating_sub(1));
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
                                &mut content_mode,
                                &mut show_search,
                                &mut search_query,
                                &mut search_dirty,
                                &mut visual_select_mode,
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

                if handle_line_filter_key(
                    key.code,
                    dialogue_count,
                    &mut line_filter_input_open,
                    &mut line_filter,
                    &mut line_filter_error,
                ) {
                    continue;
                }

                match key.code {
                    KeyCode::Char('/') => {
                        if sessions.iter().any(|session| session.load.is_some()) {
                            let indexed_sessions = workspace_sessions_for_sources(
                                &all_sessions,
                                &sources,
                                &selected_sources,
                            )
                            .into_iter()
                            .enumerate()
                            .collect::<Vec<_>>();
                            let indices = indexed_sessions
                                .iter()
                                .map(|(idx, _)| *idx)
                                .collect::<Vec<_>>();
                            load_workspace_sessions_for_indices(&mut all_sessions, &indices)?;
                            search_index = WorkspaceSearchIndex::new(&all_sessions);
                        }
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
                    KeyCode::Char('i') if dialogue_count > 0 => {
                        load_workspace_dialogue_sessions_with_cache(
                            &mut all_sessions,
                            &mut sessions,
                            session_idx,
                            &selected_sessions,
                        )?;
                        let dialogues = workspace_dialogues_for_sessions(
                            &sessions,
                            session_idx,
                            &selected_sessions,
                        );
                        let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                        match workspace_picked_content_for_copy_with_line_filter(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                            WorkspaceCopyShortcut::Input,
                            line_filter_spec(&line_filter),
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
                    }
                    KeyCode::Char('o') if dialogue_count > 0 => {
                        load_workspace_dialogue_sessions_with_cache(
                            &mut all_sessions,
                            &mut sessions,
                            session_idx,
                            &selected_sessions,
                        )?;
                        let dialogues = workspace_dialogues_for_sessions(
                            &sessions,
                            session_idx,
                            &selected_sessions,
                        );
                        let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                        match workspace_picked_content_for_copy_with_line_filter(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                            WorkspaceCopyShortcut::Output,
                            line_filter_spec(&line_filter),
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
                    }
                    KeyCode::Char('y') if dialogue_count > 0 => {
                        load_workspace_dialogue_sessions_with_cache(
                            &mut all_sessions,
                            &mut sessions,
                            session_idx,
                            &selected_sessions,
                        )?;
                        let dialogues = workspace_dialogues_for_sessions(
                            &sessions,
                            session_idx,
                            &selected_sessions,
                        );
                        let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                        match workspace_picked_content_for_copy_with_line_filter(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                            WorkspaceCopyShortcut::Block,
                            line_filter_spec(&line_filter),
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
                    }
                    KeyCode::Char('c') if dialogue_count > 0 => {
                        load_workspace_dialogue_sessions_with_cache(
                            &mut all_sessions,
                            &mut sessions,
                            session_idx,
                            &selected_sessions,
                        )?;
                        let dialogues = workspace_dialogues_for_sessions(
                            &sessions,
                            session_idx,
                            &selected_sessions,
                        );
                        let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                        match workspace_picked_content_for_copy_with_line_filter(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                            WorkspaceCopyShortcut::Command,
                            line_filter_spec(&line_filter),
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
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
                        let size = terminal.size()?;
                        let layout = workspace_layout(
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                            focus,
                            fullscreen,
                        );
                        content_scroll = workspace_content_line_count(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                            layout.content,
                            content_mode,
                        )
                        .saturating_sub(1);
                    }
                    KeyCode::Char('r') if focus == WorkspaceFocus::Content => {
                        content_mode = content_mode.toggle();
                    }
                    KeyCode::Char('v') if focus == WorkspaceFocus::Content => {
                        let size = terminal.size()?;
                        let layout = workspace_layout(
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                            focus,
                            fullscreen,
                        );
                        enter_visual_select_mode(
                            &mut visual_select_mode,
                            &mut content_scroll,
                            layout.content,
                            &workspace_display_text(&dialogues, &selected_dialogues, dialogue_idx),
                            content_mode,
                        );
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
                        load_workspace_dialogue_sessions_with_cache(
                            &mut all_sessions,
                            &mut sessions,
                            session_idx,
                            &selected_sessions,
                        )?;
                        let dialogues = workspace_dialogues_for_sessions(
                            &sessions,
                            session_idx,
                            &selected_sessions,
                        );
                        let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
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
                            load_workspace_session_at_with_cache(
                                &mut all_sessions,
                                &mut sessions,
                                session_idx,
                            )?;
                            let dialogues = workspace_dialogues_for_sessions(
                                &sessions,
                                session_idx,
                                &selected_sessions,
                            );
                            if !dialogues.is_empty() {
                                set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Dialogues);
                            }
                        }
                        WorkspaceFocus::Dialogues => {
                            load_workspace_dialogue_sessions_with_cache(
                                &mut all_sessions,
                                &mut sessions,
                                session_idx,
                                &selected_sessions,
                            )?;
                            let dialogues = workspace_dialogues_for_sessions(
                                &sessions,
                                session_idx,
                                &selected_sessions,
                            );
                            let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                            match workspace_picked_content_with_line_filter(
                                &dialogues,
                                &selected_dialogues,
                                dialogue_idx,
                                line_filter_spec(&line_filter),
                            ) {
                                Ok(picked) => return Ok(picked),
                                Err(err) => {
                                    line_filter_input_open = true;
                                    line_filter_error = Some(err.to_string());
                                }
                            }
                        }
                        WorkspaceFocus::Content => {
                            load_workspace_dialogue_sessions_with_cache(
                                &mut all_sessions,
                                &mut sessions,
                                session_idx,
                                &selected_sessions,
                            )?;
                            let dialogues = workspace_dialogues_for_sessions(
                                &sessions,
                                session_idx,
                                &selected_sessions,
                            );
                            let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                            match workspace_picked_content_with_line_filter(
                                &dialogues,
                                &selected_dialogues,
                                dialogue_idx,
                                line_filter_spec(&line_filter),
                            ) {
                                Ok(picked) => return Ok(picked),
                                Err(err) => {
                                    line_filter_input_open = true;
                                    line_filter_error = Some(err.to_string());
                                }
                            }
                        }
                    },
                    _ => {}
                }
            }
            Event::Mouse(mouse) if visual_select_mode.is_some() => {
                let size = terminal.size()?;
                let layout = workspace_layout(
                    ratatui::layout::Rect::new(0, 0, size.width, size.height),
                    focus,
                    fullscreen,
                );
                let text = workspace_display_text(&dialogues, &selected_dialogues, dialogue_idx);
                handle_visual_select_mouse(
                    visual_select_mode.as_mut().expect("checked above"),
                    mouse.kind,
                    mouse.modifiers,
                    mouse.column,
                    mouse.row,
                    VisualContentContext {
                        area: layout.content,
                        text: &text,
                        mode: content_mode,
                        scroll: content_scroll,
                    },
                );
            }
            Event::Mouse(mouse) if show_help && !show_search => match mouse.kind {
                MouseEventKind::ScrollUp => scroll_list_state_up(&mut help_state),
                MouseEventKind::ScrollDown => {
                    scroll_list_state_down(&mut help_state, workspace_help_entries().len())
                }
                _ => {}
            },
            Event::Mouse(mouse) if !show_help && !show_search => {
                let size = terminal.size()?;
                let layout = workspace_layout(
                    ratatui::layout::Rect::new(0, 0, size.width, size.height),
                    focus,
                    fullscreen,
                );
                match mouse.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        if let Some(scroll_focus) =
                            workspace_hit_test(layout, mouse.column, mouse.row)
                        {
                            apply_workspace_mouse_scroll(
                                scroll_focus,
                                matches!(mouse.kind, MouseEventKind::ScrollUp),
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
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(clicked_focus) =
                            workspace_hit_test(layout, mouse.column, mouse.row)
                        {
                            set_focus(&mut focus, &mut fullscreen, clicked_focus);
                            if clicked_focus == WorkspaceFocus::Dialogues {
                                let session_idx = selected_index(&session_state)
                                    .min(sessions.len().saturating_sub(1));
                                load_workspace_dialogue_sessions_with_cache(
                                    &mut all_sessions,
                                    &mut sessions,
                                    session_idx,
                                    &selected_sessions,
                                )?;
                            }
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
                                WorkspaceFocus::Content => {
                                    let dialogue_idx = selected_index(&dialogue_state)
                                        .min(dialogues.len().saturating_sub(1));
                                    let text = workspace_display_text(
                                        &dialogues,
                                        &selected_dialogues,
                                        dialogue_idx,
                                    );
                                    if let Some(target) = content_link_at(
                                        layout.content,
                                        &text,
                                        content_scroll,
                                        content_mode,
                                        mouse.column,
                                        mouse.row,
                                    ) {
                                        let _ = open_link_target(&target);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn enter_visual_select_mode(
    visual_select_mode: &mut Option<VisualSelectMode>,
    content_scroll: &mut usize,
    content_area: ratatui::layout::Rect,
    text: &str,
    mode: ContentViewMode,
) {
    let position = clamp_content_position(
        content_area,
        text,
        mode,
        ContentPosition {
            line: *content_scroll,
            column: 0,
        },
    );
    *content_scroll = position.line;
    *visual_select_mode = Some(VisualSelectMode {
        selection: ContentSelection {
            anchor: position,
            cursor: position,
            kind: ContentSelectionKind::Linear,
        },
        dragging: false,
    });
}

#[allow(clippy::too_many_arguments)]
fn handle_visual_select_key(
    key: KeyCode,
    modifiers: KeyModifiers,
    mode: &mut VisualSelectMode,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    content_scroll: &mut usize,
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
) -> Result<Option<WorkspacePickedContent>> {
    match key {
        KeyCode::Esc | KeyCode::Char('v') => return Ok(None),
        KeyCode::Enter | KeyCode::Char('y') => {
            return Ok(Some(workspace_picked_content_for_visual_selection(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                content_area,
                text,
                content_mode,
                mode.selection,
            )));
        }
        KeyCode::Left | KeyCode::Char('h') => move_visual_cursor(
            mode,
            content_area,
            text,
            content_mode,
            content_scroll,
            -1,
            0,
        ),
        KeyCode::Right | KeyCode::Char('l') => {
            move_visual_cursor(mode, content_area, text, content_mode, content_scroll, 1, 0)
        }
        KeyCode::Up | KeyCode::Char('k') => move_visual_cursor(
            mode,
            content_area,
            text,
            content_mode,
            content_scroll,
            0,
            -1,
        ),
        KeyCode::Down | KeyCode::Char('j') => {
            move_visual_cursor(mode, content_area, text, content_mode, content_scroll, 0, 1)
        }
        KeyCode::Home | KeyCode::Char('0') => {
            mode.selection.cursor.column = 0;
        }
        KeyCode::End | KeyCode::Char('$') => {
            mode.selection.cursor = clamp_content_position(
                content_area,
                text,
                content_mode,
                ContentPosition {
                    line: mode.selection.cursor.line,
                    column: usize::MAX,
                },
            );
        }
        KeyCode::PageDown | KeyCode::Char('d')
            if key == KeyCode::PageDown || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            move_visual_cursor(
                mode,
                content_area,
                text,
                content_mode,
                content_scroll,
                0,
                10,
            )
        }
        KeyCode::PageUp | KeyCode::Char('u')
            if key == KeyCode::PageUp || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            move_visual_cursor(
                mode,
                content_area,
                text,
                content_mode,
                content_scroll,
                0,
                -10,
            )
        }
        _ => {}
    }
    ensure_visual_cursor_visible(mode, content_area, text, content_mode, content_scroll);
    Ok(None)
}

fn move_visual_cursor(
    mode: &mut VisualSelectMode,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    content_scroll: &mut usize,
    column_delta: isize,
    line_delta: isize,
) {
    let cursor = mode.selection.cursor;
    let line = cursor.line.saturating_add_signed(line_delta);
    let column = cursor.column.saturating_add_signed(column_delta);
    mode.selection.cursor = clamp_content_position(
        content_area,
        text,
        content_mode,
        ContentPosition { line, column },
    );
    ensure_visual_cursor_visible(mode, content_area, text, content_mode, content_scroll);
}

fn ensure_visual_cursor_visible(
    mode: &VisualSelectMode,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    content_scroll: &mut usize,
) {
    let text_area = content_text_area(content_area, text, content_mode);
    let height = text_area.height as usize;
    if height == 0 {
        return;
    }
    let cursor_line = mode.selection.cursor.line;
    if cursor_line < *content_scroll {
        *content_scroll = cursor_line;
    } else if cursor_line >= content_scroll.saturating_add(height) {
        *content_scroll = cursor_line.saturating_add(1).saturating_sub(height);
    }
}

fn handle_visual_select_mouse(
    mode: &mut VisualSelectMode,
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    column: u16,
    row: u16,
    content: VisualContentContext<'_>,
) {
    let Some(position) = content_position_in_text_row(
        content.area,
        content.text,
        content.scroll,
        content.mode,
        column,
        row,
    ) else {
        return;
    };

    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            mode.selection = ContentSelection {
                anchor: position,
                cursor: position,
                kind: mouse_selection_kind(modifiers),
            };
            mode.dragging = true;
        }
        MouseEventKind::Drag(MouseButton::Left) if mode.dragging => {
            mode.selection.cursor = position;
            if modifiers.contains(KeyModifiers::CONTROL) {
                mode.selection.kind = ContentSelectionKind::Block;
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            mode.selection.cursor = position;
            mode.dragging = false;
        }
        _ => {}
    }
}

fn mouse_selection_kind(modifiers: KeyModifiers) -> ContentSelectionKind {
    if modifiers.contains(KeyModifiers::CONTROL) {
        ContentSelectionKind::Block
    } else {
        ContentSelectionKind::Linear
    }
}

fn workspace_picked_content_for_visual_selection(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    selection: ContentSelection,
) -> WorkspacePickedContent {
    let source = workspace_picked_content(dialogues, selected_dialogues, dialogue_idx).source;
    let plain = selected_content_text(content_area, text, content_mode, selection);
    WorkspacePickedContent {
        source,
        units: vec![crate::tui::workspace::TextPair {
            ansi: plain.clone(),
            plain,
        }],
        selection: CommandSelection::RecentExplicit(vec![1]),
    }
}

fn scroll_list_state_up(state: &mut ListState) {
    for _ in 0..MOUSE_SCROLL_LINES {
        state.select(Some(selected_index(state).saturating_sub(1)));
    }
}

fn scroll_list_state_down(state: &mut ListState, len: usize) {
    for _ in 0..MOUSE_SCROLL_LINES {
        let next = (selected_index(state) + 1).min(len.saturating_sub(1));
        state.select((len > 0).then_some(next));
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_workspace_mouse_scroll(
    focus: WorkspaceFocus,
    scroll_up: bool,
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
    for _ in 0..MOUSE_SCROLL_LINES {
        if scroll_up {
            move_workspace_cursor_up(
                focus,
                sources,
                sessions,
                dialogue_count,
                selected_sessions,
                source_state,
                session_state,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scroll,
            );
        } else {
            move_workspace_cursor_down(
                focus,
                sources,
                sessions,
                dialogue_count,
                selected_sessions,
                source_state,
                session_state,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scroll,
            );
        }
    }
}

fn load_workspace_session_at_with_cache(
    all_sessions: &mut [WorkspaceSession],
    sessions: &mut [WorkspaceSession],
    idx: usize,
) -> Result<()> {
    let load_path = sessions
        .get(idx)
        .and_then(|session| session.load.as_ref())
        .map(|load| load.path.clone());
    load_workspace_session_at(sessions, idx)?;
    if let (Some(load_path), Some(loaded)) = (
        load_path,
        sessions.get(idx).filter(|session| session.load.is_none()),
    ) {
        if let Some(cached) = all_sessions.iter_mut().find(|session| {
            session
                .load
                .as_ref()
                .is_some_and(|load| load.path == load_path)
        }) {
            *cached = loaded.clone();
        }
    }
    Ok(())
}

fn load_workspace_dialogue_sessions_with_cache(
    all_sessions: &mut [WorkspaceSession],
    sessions: &mut [WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
) -> Result<()> {
    let selected = selected_sessions
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    let indices = if selected.is_empty() {
        vec![session_idx]
    } else {
        selected
    };
    for idx in indices {
        load_workspace_session_at_with_cache(all_sessions, sessions, idx)?;
    }
    Ok(())
}

fn workspace_display_text(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
) -> String {
    if dialogues.is_empty() {
        return "<empty>".to_string();
    }

    workspace_picked_content(dialogues, selected_dialogues, dialogue_idx)
        .units
        .into_iter()
        .map(|unit| unit.plain)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn open_link_target(target: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(target).spawn()?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn()?;
    }

    Ok(())
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

#[allow(clippy::too_many_arguments)]
fn apply_workspace_help_action(
    action: WorkspaceHelpAction,
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    sources: &[WorkspaceSource],
    source_state: &mut ListState,
    selected_sources: &mut [bool],
    selected_sessions: &mut Vec<bool>,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scroll: &mut usize,
    content_mode: &mut ContentViewMode,
    show_search: &mut bool,
    search_query: &mut String,
    search_dirty: &mut bool,
    visual_select_mode: &mut Option<VisualSelectMode>,
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
        WorkspaceHelpAction::ToggleContentMode if *focus == WorkspaceFocus::Content => {
            *content_mode = content_mode.toggle();
        }
        WorkspaceHelpAction::VisualTextSelect if *focus == WorkspaceFocus::Content => {
            let size = terminal.size()?;
            let layout = workspace_layout(
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                *focus,
                *fullscreen,
            );
            enter_visual_select_mode(
                visual_select_mode,
                content_scroll,
                layout.content,
                &workspace_display_text(dialogues, selected_dialogues, dialogue_idx),
                *content_mode,
            );
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
        WorkspaceHelpAction::CopyInput if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Input,
            )));
        }
        WorkspaceHelpAction::CopyOutput if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Output,
            )));
        }
        WorkspaceHelpAction::CopyBlock if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Block,
            )));
        }
        WorkspaceHelpAction::CopyCommand if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Command,
            )));
        }
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
    workspace_picked_content_with_line_filter(dialogues, selected_dialogues, dialogue_idx, None)
        .expect("workspace copy without a line filter should not fail")
}

fn workspace_picked_content_with_line_filter(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    line_filter: Option<&str>,
) -> Result<WorkspacePickedContent> {
    workspace_picked_content_for_copy_with_line_filter(
        dialogues,
        selected_dialogues,
        dialogue_idx,
        WorkspaceCopyShortcut::Displayed,
        line_filter,
    )
}

#[derive(Clone, Copy)]
enum WorkspaceCopyShortcut {
    Displayed,
    Input,
    Output,
    Block,
    Command,
}

fn workspace_picked_content_for_copy(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    shortcut: WorkspaceCopyShortcut,
) -> WorkspacePickedContent {
    workspace_picked_content_for_copy_with_line_filter(
        dialogues,
        selected_dialogues,
        dialogue_idx,
        shortcut,
        None,
    )
    .expect("workspace copy without a line filter should not fail")
}

fn workspace_picked_content_for_copy_with_line_filter(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    shortcut: WorkspaceCopyShortcut,
    line_filter: Option<&str>,
) -> Result<WorkspacePickedContent> {
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
        .map(|dialogue| match shortcut {
            WorkspaceCopyShortcut::Displayed => dialogue.unit.clone(),
            WorkspaceCopyShortcut::Input => dialogue.copy.input.clone(),
            WorkspaceCopyShortcut::Output => dialogue.copy.output.clone(),
            WorkspaceCopyShortcut::Block => dialogue.copy.block.clone(),
            WorkspaceCopyShortcut::Command => dialogue.copy.command.clone(),
        })
        .collect::<Vec<_>>();
    let units = apply_workspace_line_filter(units, line_filter)?;
    let selection = CommandSelection::RecentExplicit((1..=units.len()).collect());
    Ok(WorkspacePickedContent {
        source: dialogues[source_idx].source,
        units,
        selection,
    })
}

fn line_filter_spec(line_filter: &str) -> Option<&str> {
    (!line_filter.is_empty()).then_some(line_filter)
}

fn apply_workspace_line_filter(
    units: Vec<crate::tui::workspace::TextPair>,
    line_filter: Option<&str>,
) -> Result<Vec<crate::tui::workspace::TextPair>> {
    let Some(spec) = line_filter else {
        return Ok(units);
    };

    units
        .into_iter()
        .map(|unit| filter_lines_by_spec(&unit, spec))
        .collect()
}

fn handle_line_filter_key(
    key: KeyCode,
    dialogue_count: usize,
    line_filter_input_open: &mut bool,
    line_filter: &mut String,
    line_filter_error: &mut Option<String>,
) -> bool {
    if *line_filter_input_open {
        match key {
            KeyCode::Char(ch) if matches!(ch, '0'..='9' | ':' | ',') => {
                line_filter.push(ch);
                *line_filter_error = None;
                return true;
            }
            KeyCode::Backspace => {
                *line_filter_error = None;
                if line_filter.pop().is_none() {
                    *line_filter_input_open = false;
                }
                return true;
            }
            KeyCode::Esc => {
                *line_filter_input_open = false;
                line_filter.clear();
                *line_filter_error = None;
                return true;
            }
            _ => {}
        }
    }

    match key {
        KeyCode::Char(':') if dialogue_count > 0 => {
            *line_filter_input_open = true;
            *line_filter_error = None;
            true
        }
        KeyCode::Esc if line_filter_error.is_some() => {
            *line_filter_error = None;
            true
        }
        _ => false,
    }
}

fn workspace_content_line_count(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    content_area: ratatui::layout::Rect,
    mode: ContentViewMode,
) -> usize {
    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return dialogues
            .get(highlighted_idx)
            .map(|dialogue| content_view_line_count(content_area, &dialogue.unit.plain, mode))
            .unwrap_or(1);
    }

    let text = selected
        .into_iter()
        .filter_map(|dialogue_idx| dialogues.get(dialogue_idx))
        .map(|dialogue| dialogue.unit.plain.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    content_view_line_count(content_area, &text, mode)
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

    if session_indices
        .iter()
        .filter_map(|idx| sessions.get(*idx))
        .any(|session| session.load.is_some())
    {
        return session_indices
            .into_iter()
            .filter_map(|idx| sessions.get(idx))
            .map(|session| WorkspaceDialogue {
                source: session.source,
                title: "<loading: press Enter to load dialogue>".to_string(),
                unit: TextPair {
                    plain: "<loading: press Enter to load dialogue>".to_string(),
                    ansi: String::new(),
                },
                copy: crate::tui::workspace::WorkspaceCopyParts::default(),
            })
            .collect();
    }

    session_indices
        .into_iter()
        .filter_map(|idx| sessions.get(idx))
        .flat_map(|session| {
            session
                .dialogue_titles
                .iter()
                .cloned()
                .enumerate()
                .map(move |(idx, title)| {
                    let unit = session.units.get(idx).cloned().unwrap_or_default();
                    let copy = session.copy_units.get(idx).cloned().unwrap_or_else(|| {
                        crate::tui::workspace::WorkspaceCopyParts::from_block(unit.clone())
                    });
                    WorkspaceDialogue {
                        source: session.source,
                        title,
                        unit,
                        copy,
                    }
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
        handle_line_filter_key, workspace_dialogue_vim_view, workspace_dialogues_for_sessions,
        workspace_picked_content, workspace_picked_content_for_copy,
        workspace_picked_content_for_copy_with_line_filter,
        workspace_picked_content_with_line_filter, WorkspaceCopyShortcut,
    };
    use crate::commands::command_block_selector::CommandSelection;
    use crate::tui::workspace::{
        TextPair, WorkspaceCopyParts, WorkspaceDialogue, WorkspaceSession, WorkspaceSource,
    };
    use crate::tui::workspace_search::{
        workspace_search_query, workspace_search_regex, WorkspaceSearchIndex, WorkspaceSearchMatch,
        WorkspaceSearchScope,
    };
    use crossterm::event::KeyCode;
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
            ref_id: "session".to_string(),
            title: "session".to_string(),
            search_title: "session".to_string(),
            units: vec![TextPair {
                plain: "first\nneedle one\nmiddle\nneedle two".to_string(),
                ansi: String::new(),
            }],
            copy_units: vec![WorkspaceCopyParts::from_block(TextPair {
                plain: "first\nneedle one\nmiddle\nneedle two".to_string(),
                ansi: String::new(),
            })],
            dialogue_titles: vec!["dialogue".to_string()],
            unit_timestamps: vec![None],
            original_dialogue_indices: vec![0],
            load: None,
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
    fn workspace_copy_shortcuts_use_structured_chat_parts_without_headings() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            title: "question".to_string(),
            unit: TextPair {
                plain: "## User\nquestion\n\n## Assistant\nanswer".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts {
                input: TextPair {
                    plain: "question".to_string(),
                    ansi: String::new(),
                },
                output: TextPair {
                    plain: "answer".to_string(),
                    ansi: String::new(),
                },
                block: TextPair {
                    plain: "question\n\nanswer".to_string(),
                    ansi: String::new(),
                },
                command: TextPair::default(),
            },
        }];

        let input = workspace_picked_content_for_copy(
            &dialogues,
            &[false],
            0,
            WorkspaceCopyShortcut::Input,
        );
        let output = workspace_picked_content_for_copy(
            &dialogues,
            &[false],
            0,
            WorkspaceCopyShortcut::Output,
        );
        let block = workspace_picked_content_for_copy(
            &dialogues,
            &[false],
            0,
            WorkspaceCopyShortcut::Block,
        );

        assert_eq!(input.units[0].plain, "question");
        assert_eq!(output.units[0].plain, "answer");
        assert_eq!(block.units[0].plain, "question\n\nanswer");
    }

    #[test]
    fn workspace_line_filter_applies_to_displayed_and_structured_copies() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            title: "question".to_string(),
            unit: TextPair {
                plain: "line 1\nline 2\nline 3".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts {
                input: TextPair {
                    plain: "ask 1\nask 2\nask 3".to_string(),
                    ansi: String::new(),
                },
                output: TextPair {
                    plain: "answer 1\nanswer 2\nanswer 3".to_string(),
                    ansi: String::new(),
                },
                block: TextPair {
                    plain: "ask 1\nask 2\nask 3\n\nanswer 1\nanswer 2\nanswer 3".to_string(),
                    ansi: String::new(),
                },
                command: TextPair::default(),
            },
        }];

        let displayed =
            workspace_picked_content_with_line_filter(&dialogues, &[false], 0, Some("2:3"))
                .unwrap();
        let input = workspace_picked_content_for_copy_with_line_filter(
            &dialogues,
            &[false],
            0,
            WorkspaceCopyShortcut::Input,
            Some("1,3"),
        )
        .unwrap();

        assert_eq!(displayed.units[0].plain, "line 2\nline 3");
        assert_eq!(input.units[0].plain, "ask 1\nask 3");
    }

    #[test]
    fn workspace_line_filter_rejects_invalid_specs() {
        let dialogues = vec![workspace_test_dialogue("d1", "alpha\nbeta\ngamma")];

        let err = workspace_picked_content_with_line_filter(&dialogues, &[false], 0, Some("x"))
            .unwrap_err();

        assert!(
            err.to_string().contains("Invalid line number"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn line_filter_key_handler_keeps_colon_inside_active_input() {
        let mut open = false;
        let mut filter = String::new();
        let mut error = None;

        assert!(handle_line_filter_key(
            KeyCode::Char(':'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));
        assert!(open);
        assert_eq!(filter, "");

        assert!(handle_line_filter_key(
            KeyCode::Char('2'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));
        assert!(handle_line_filter_key(
            KeyCode::Char(':'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));
        assert!(handle_line_filter_key(
            KeyCode::Char('3'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));

        assert_eq!(filter, "2:3");
        assert!(open);
    }

    #[test]
    fn workspace_command_shortcut_uses_terminal_command_without_prompt() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::Terminal,
            title: "cargo test".to_string(),
            unit: TextPair {
                plain: "PS C:\\repo> cargo test\nok".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts {
                input: TextPair {
                    plain: "PS C:\\repo> cargo test".to_string(),
                    ansi: String::new(),
                },
                output: TextPair {
                    plain: "ok".to_string(),
                    ansi: String::new(),
                },
                block: TextPair {
                    plain: "PS C:\\repo> cargo test\nok".to_string(),
                    ansi: String::new(),
                },
                command: TextPair {
                    plain: "cargo test".to_string(),
                    ansi: "cargo test".to_string(),
                },
            },
        }];

        let picked = workspace_picked_content_for_copy(
            &dialogues,
            &[false],
            0,
            WorkspaceCopyShortcut::Command,
        );

        assert_eq!(picked.units[0].plain, "cargo test");
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
            copy: WorkspaceCopyParts::from_block(TextPair {
                plain: "line1\nline2\nline3\nline4".to_string(),
                ansi: "line1\nline2\nline3\nline4".to_string(),
            }),
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
            ref_id: title.to_string(),
            title: title.to_string(),
            search_title: title.to_string(),
            units: dialogue_titles
                .iter()
                .map(|dialogue_title| TextPair {
                    plain: format!("{title}:{dialogue_title}"),
                    ansi: format!("{title}:{dialogue_title}"),
                })
                .collect(),
            copy_units: dialogue_titles
                .iter()
                .map(|dialogue_title| {
                    WorkspaceCopyParts::from_block(TextPair {
                        plain: format!("{title}:{dialogue_title}"),
                        ansi: format!("{title}:{dialogue_title}"),
                    })
                })
                .collect(),
            dialogue_titles: dialogue_titles
                .iter()
                .map(|dialogue_title| dialogue_title.to_string())
                .collect(),
            unit_timestamps: vec![None; dialogue_titles.len()],
            original_dialogue_indices: (0..dialogue_titles.len()).collect(),
            load: None,
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
            copy: WorkspaceCopyParts::from_block(TextPair {
                plain: plain.to_string(),
                ansi: plain.to_string(),
            }),
        }
    }
}
