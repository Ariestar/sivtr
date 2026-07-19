use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::widgets::ListState;
use std::path::PathBuf;

use crate::tui::content_view::{content_link_at, ContentViewMode};
use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    can_open_dialogue_vim, panel_inner_rows, render_workspace, selected_index, workspace_content_text,
    workspace_help_entries, workspace_hit_test, workspace_layout, WorkspaceFocus,
    WorkspacePickedContent, WorkspaceSearchView, WorkspaceSource, WorkspaceView,
};
use crate::tui::workspace_search::{
    workspace_search_has_query, workspace_search_scope, WorkspaceSearchIndex, WorkspaceSearchOutput,
};

use super::content::{
    active_workspace_content_at, apply_dialogue_range_selection, dialogue_text_vim_view,
    handle_line_filter_key, line_filter_spec, workspace_content_line_count,
    workspace_dialogues_for_sessions, workspace_picked_content_for_copy_with_line_filter,
    workspace_picked_content_with_line_filter, workspace_search_target_ref, WorkspaceCopyShortcut,
};

use super::help::{apply_workspace_help_action, set_focus, toggle_fullscreen};
use super::load::{
    body_keep_set, collect_ready_sessions, SessionViewport, SourceLoadPump, SourceLoadState,
};
use super::nav::{
    clamp_list_state, move_workspace_cursor_down, move_workspace_cursor_up, open_link_target,
    reset_workspace_after_source_change, reset_workspace_dialogue_state,
    reset_workspace_search_state, resize_workspace_dialogue_selection, row_list_index,
    source_inline_index,
};
use super::selection::{
    has_selected_sessions, refresh_next_level, select_sources, WorkspaceSourceSelection,
};
use super::vim::open_vim_view;
use super::visual::{
    apply_workspace_mouse_scroll, enter_visual_select_mode, handle_visual_select_key,
    handle_visual_select_mouse, scroll_list_state_down, scroll_list_state_up, VisualContentContext,
    VisualSelectMode,
};
use super::PICK_CANCELLED_MESSAGE;

pub(crate) fn run(
    terminal: &mut crate::tui::terminal::Tui,
    sources: Vec<WorkspaceSource>,
    mut source_states: Vec<SourceLoadState>,
    selected_sources: Vec<bool>,
    cwd: PathBuf,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    assert_eq!(sources.len(), selected_sources.len());
    assert_eq!(sources.len(), source_states.len());
    let mut selected_sources = selected_sources;
    let mut session_state = ListState::default();
    let mut source_state = ListState::default();
    let mut dialogue_state = ListState::default();
    let mut help_state = ListState::default();
    help_state.select(Some(0));
    let mut focus = initial_focus;
    let mut load_pump = SourceLoadPump::new(sources.len(), cwd.clone());
    // Bootstrap with a conservative viewport; first draw refines from layout.
    let bootstrap = SessionViewport {
        first_visible: 0,
        visible_rows: 24,
    };
    load_pump.kick(
        &sources,
        &selected_sources,
        &mut source_states,
        bootstrap,
        true,
    );
    let mut all_sessions = collect_ready_sessions(&sources, &selected_sources, &source_states);
    let mut sessions = all_sessions.clone();
    clamp_list_state(&mut source_state, sources.len());
    clamp_list_state(&mut session_state, sessions.len());
    clamp_list_state(&mut dialogue_state, 0);
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
    let mut loading_tick = 0u8;

    loop {
        // Non-blocking: apply completed source loads before drawing.
        if load_pump.drain(&mut source_states) {
            all_sessions = collect_ready_sessions(&sources, &selected_sources, &source_states);
            search_index = WorkspaceSearchIndex::new(&all_sessions);
            search_dirty = true;
        }
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
            all_sessions.clone()
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
        session_state.select((!sessions.is_empty()).then_some(session_idx));

        // Viewport from current terminal size (sessions panel inner rows).
        let size = terminal.size()?;
        let layout = workspace_layout(
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
            focus,
            fullscreen,
        );
        let session_viewport = SessionViewport::from_panel(
            session_state.offset(),
            panel_inner_rows(layout.sessions),
        );

        // Drop caches for unselected sources; grow meta to cover the viewport.
        load_pump.drop_unselected(&selected_sources, &mut source_states);
        if !search_has_query {
            load_pump.ensure_meta_window(
                &sources,
                &selected_sources,
                &mut source_states,
                session_viewport,
                sessions.len(),
            );
        }

        // Sliding body window: hydrate keep-set, evict everything else.
        let keep = body_keep_set(
            &sources,
            &sessions,
            session_idx,
            &selected_sessions,
            1, // one neighbor each side for smooth j/k
        );
        load_pump.sync_bodies(&sources, &mut source_states, &keep);

        // Refresh merged list after possible meta/body updates applied next drain;
        // re-collect so dialogues see just-evicted/hydrated state from prior frame.
        all_sessions = collect_ready_sessions(&sources, &selected_sources, &source_states);
        sessions = if search_has_query {
            search_output.sessions.clone()
        } else {
            all_sessions.clone()
        };
        if selected_sessions.len() != sessions.len() {
            selected_sessions.resize(sessions.len(), false);
        }
        let session_idx = selected_index(&session_state).min(sessions.len().saturating_sub(1));
        session_state.select((!sessions.is_empty()).then_some(session_idx));

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
                content_scroll = matched.content_scroll_index().min(
                    dialogue
                        .content_line_count(content_mode, None)
                        .saturating_sub(1),
                );
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
            let content_at = active_workspace_content_at(
                search_has_query,
                &search_output,
                search_cursor,
                session_idx,
                &selected_dialogues,
                dialogue_idx,
            );
            content_scroll = content_scroll.min(
                workspace_content_line_count(
                    &dialogues,
                    &selected_dialogues,
                    dialogue_idx,
                    content_at,
                    layout.content,
                    content_mode,
                )
                .saturating_sub(1),
            );
        }
        let active_content_at = active_workspace_content_at(
            search_has_query,
            &search_output,
            search_cursor,
            session_idx,
            &selected_dialogues,
            dialogue_idx,
        );

        let source_markers = source_states
            .iter()
            .map(SourceLoadState::marker)
            .collect::<Vec<_>>();
        terminal.draw(|frame| {
            render_workspace(
                frame,
                WorkspaceView {
                    sources: &sources,
                    selected_sources: &selected_sources,
                    source_markers: &source_markers,
                    loading_tick,
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
                    content_at: active_content_at,
                    show_help,
                    help_state: &help_state,
                    search: (show_search || search_has_query).then_some(WorkspaceSearchView {
                        query: &search_query,
                        scope: workspace_search_scope(&search_query),
                        result_count: sessions.len(),
                        current_match: (!search_output.matches.is_empty()).then_some(search_cursor),
                        match_count: search_output.matches.len(),
                        current_target: search_output
                            .matches
                            .get(search_cursor)
                            .and_then(|matched| workspace_search_target_ref(&sessions, matched))
                            .map(|work_ref| work_ref.to_string()),
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

        // Poll so background loads can repaint without waiting for a key.
        if !event::poll(std::time::Duration::from_millis(100))? {
            if source_states.iter().any(SourceLoadState::is_fetching) {
                loading_tick = loading_tick.wrapping_add(1);
            }
            continue;
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
                    let text = workspace_content_text(
                        &dialogues,
                        &selected_dialogues,
                        dialogue_idx,
                        content_mode,
                        active_content_at,
                    );
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
                                active_content_at,
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
                            None,
                            content_mode,
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
                    }
                    KeyCode::Char('o') if dialogue_count > 0 => {
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
                            None,
                            content_mode,
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
                    }
                    KeyCode::Char('y') if dialogue_count > 0 => {
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
                            None,
                            content_mode,
                        ) {
                            Ok(picked) => return Ok(picked),
                            Err(err) => {
                                line_filter_input_open = true;
                                line_filter_error = Some(err.to_string());
                            }
                        }
                    }
                    KeyCode::Char('c') if dialogue_count > 0 => {
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
                            None,
                            content_mode,
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
                        {
                            let size = terminal.size()?;
                            let layout = workspace_layout(
                                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                focus,
                                fullscreen,
                            );
                            let viewport = SessionViewport::from_panel(
                                session_state.offset(),
                                panel_inner_rows(layout.sessions),
                            );
                            load_pump.kick(
                                &sources,
                                &selected_sources,
                                &mut source_states,
                                viewport,
                                true,
                            );
                        };
                        all_sessions =
                            collect_ready_sessions(&sources, &selected_sources, &source_states);
                        search_index = WorkspaceSearchIndex::new(&all_sessions);
                        search_dirty = true;
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
                        {
                            let size = terminal.size()?;
                            let layout = workspace_layout(
                                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                focus,
                                fullscreen,
                            );
                            let viewport = SessionViewport::from_panel(
                                session_state.offset(),
                                panel_inner_rows(layout.sessions),
                            );
                            load_pump.kick(
                                &sources,
                                &selected_sources,
                                &mut source_states,
                                viewport,
                                true,
                            );
                        };
                        all_sessions =
                            collect_ready_sessions(&sources, &selected_sources, &source_states);
                        search_index = WorkspaceSearchIndex::new(&all_sessions);
                        search_dirty = true;
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
                        {
                            let size = terminal.size()?;
                            let layout = workspace_layout(
                                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                focus,
                                fullscreen,
                            );
                            let viewport = SessionViewport::from_panel(
                                session_state.offset(),
                                panel_inner_rows(layout.sessions),
                            );
                            load_pump.kick(
                                &sources,
                                &selected_sources,
                                &mut source_states,
                                viewport,
                                true,
                            );
                        };
                        all_sessions =
                            collect_ready_sessions(&sources, &selected_sources, &source_states);
                        search_index = WorkspaceSearchIndex::new(&all_sessions);
                        search_dirty = true;
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
                    KeyCode::Char('R') => {
                        let size = terminal.size()?;
                        let layout = workspace_layout(
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                            focus,
                            fullscreen,
                        );
                        let viewport = SessionViewport::from_panel(
                            session_state.offset(),
                            panel_inner_rows(layout.sessions),
                        );
                        refresh_next_level(
                            focus,
                            &sources,
                            &selected_sources,
                            &source_state,
                            &sessions,
                            &selected_sessions,
                            &session_state,
                            &mut source_states,
                            &mut load_pump,
                            &mut all_sessions,
                            &mut search_index,
                            &mut search_dirty,
                            viewport,
                        );
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
                    KeyCode::Up | KeyCode::Char('k') => {
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
                    KeyCode::Down | KeyCode::Char('j') => {
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
                            active_content_at,
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
                            &workspace_content_text(
                                &dialogues,
                                &selected_dialogues,
                                dialogue_idx,
                                content_mode,
                                active_content_at,
                            ),
                            content_mode,
                        );
                    }
                    KeyCode::Char(' ') => match focus {
                        WorkspaceFocus::Source => {
                            let source_idx = selected_index(&source_state);
                            if let Some(selected) = selected_sources.get_mut(source_idx) {
                                *selected = !*selected;
                            }
                            {
                            let size = terminal.size()?;
                            let layout = workspace_layout(
                                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                focus,
                                fullscreen,
                            );
                            let viewport = SessionViewport::from_panel(
                                session_state.offset(),
                                panel_inner_rows(layout.sessions),
                            );
                            load_pump.kick(
                                &sources,
                                &selected_sources,
                                &mut source_states,
                                viewport,
                                true,
                            );
                        };
                            all_sessions =
                                collect_ready_sessions(&sources, &selected_sources, &source_states);
                            search_index = WorkspaceSearchIndex::new(&all_sessions);
                            search_dirty = true;
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
                        let dialogues = workspace_dialogues_for_sessions(
                            &sessions,
                            session_idx,
                            &selected_sessions,
                        );
                        let dialogue_idx = dialogue_idx.min(dialogues.len().saturating_sub(1));
                        let view = dialogue_text_vim_view(workspace_content_text(
                            &dialogues,
                            &selected_dialogues,
                            dialogue_idx,
                            content_mode,
                            active_content_at,
                        ));
                        restore_tui(terminal)?;
                        open_vim_view(&view)?;
                        *terminal = init_tui()?;
                    }
                    KeyCode::Enter => match focus {
                        WorkspaceFocus::Source => {
                            set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Sessions);
                        }
                        WorkspaceFocus::Sessions => {
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
                                active_content_at,
                            ) {
                                Ok(picked) => return Ok(picked),
                                Err(err) => {
                                    line_filter_input_open = true;
                                    line_filter_error = Some(err.to_string());
                                }
                            }
                        }
                        WorkspaceFocus::Content => {
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
                                active_content_at,
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
                let text = workspace_content_text(
                    &dialogues,
                    &selected_dialogues,
                    dialogue_idx,
                    content_mode,
                    active_content_at,
                );
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
                                    let text = workspace_content_text(
                                        &dialogues,
                                        &selected_dialogues,
                                        dialogue_idx,
                                        content_mode,
                                        active_content_at,
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











































#[cfg(test)]
mod tests {
    use super::super::content::{
        handle_line_filter_key, workspace_dialogue_vim_view, workspace_dialogues_for_sessions,
        workspace_picked_content, workspace_picked_content_for_copy,
        workspace_picked_content_for_copy_with_line_filter, workspace_picked_content_with_line_filter,
        workspace_search_target_ref, WorkspaceCopyShortcut,
    };
    use super::super::nav::{clamp_list_state, move_workspace_cursor_up};
    use crate::commands::select::CommandSelection;
    use crate::tui::content_view::ContentViewMode;
    use crate::tui::workspace::{
        TextPair, WorkspaceCopyParts, WorkspaceDialogue, WorkspaceFocus, WorkspaceSession,
        WorkspaceSource, WorkspaceSourceKind,
    };
    use crate::tui::workspace_search::{
        workspace_search_query, workspace_search_regex, WorkspaceSearchIndex, WorkspaceSearchMatch,
        WorkspaceSearchScope,
    };
    use crossterm::event::KeyCode;
    use ratatui::widgets::ListState;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{WorkAt, WorkRef};
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartIo, WorkPartKind, WorkRecord, WorkRecordKind,
        WorkSessionRef, WorkSource, WorkTime, RECORD_SCHEMA_VERSION,
    };
    use std::time::SystemTime;

    #[test]
    fn workspace_dialogues_follow_current_session_without_session_selection() {
        let sessions = vec![
            workspace_test_session("new", WorkspaceSource::agent(AgentProvider::Codex), &["n1"]),
            workspace_test_session(
                "old",
                WorkspaceSource::agent(AgentProvider::Claude),
                &["o1"],
            ),
        ];

        let dialogues = workspace_dialogues_for_sessions(&sessions, 1, &[false, false]);

        assert_eq!(dialogues.len(), 1);
        assert_eq!(dialogues[0].title, "o1");
        assert!(dialogues[0]
            .content_text(ContentViewMode::Reading, None)
            .contains("old:o1"));
        assert_eq!(
            dialogues[0].work_ref.as_ref().unwrap().to_string(),
            "claude/test/1"
        );
    }

    #[test]
    fn workspace_dialogues_aggregate_selected_sessions() {
        let sessions = vec![
            workspace_test_session(
                "codex session",
                WorkspaceSource::agent(AgentProvider::Codex),
                &["c1", "c2"],
            ),
            workspace_test_session(
                "claude session",
                WorkspaceSource::agent(AgentProvider::Claude),
                &["a1"],
            ),
        ];

        let dialogues = workspace_dialogues_for_sessions(&sessions, 0, &[true, true]);

        assert_eq!(dialogues.len(), 3);
        assert_eq!(dialogues[0].title, "c1");
        assert_eq!(dialogues[1].title, "c2");
        assert_eq!(dialogues[2].title, "a1");
        let texts: Vec<_> = dialogues
            .iter()
            .map(|dialogue| dialogue.content_text(ContentViewMode::Reading, None))
            .collect();
        assert!(texts[0].contains("codex session:c1"));
        assert!(texts[1].contains("codex session:c2"));
        assert!(texts[2].contains("claude session:a1"));
        assert_eq!(
            dialogues
                .iter()
                .map(|dialogue| dialogue.work_ref.as_ref().unwrap().to_string())
                .collect::<Vec<_>>(),
            vec!["codex/test/1", "codex/test/2", "claude/test/1"]
        );
    }

    #[test]
    fn workspace_search_defaults_to_dialogue_content() {
        let sessions = vec![
            workspace_test_session(
                "alpha session",
                WorkspaceSource::agent(AgentProvider::Codex),
                &["camera"],
            ),
            workspace_test_session(
                "target session",
                WorkspaceSource::agent(AgentProvider::Claude),
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
            WorkspaceSource::agent(AgentProvider::Claude)
        );
        assert_eq!(output.sessions[0].title, "target session");
        assert_eq!(output.sessions[0].records[0].title, "lighting");
        assert_eq!(
            output.sessions[0].records[0]
                .copy_text(sivtr_core::record::RecordTextMode::Combined, false)
                .plain,
            "target session:lighting"
        );
        assert_eq!(output.matches.len(), 1);
    }

    #[test]
    fn workspace_search_prefixes_select_session_or_dialogue_scope() {
        let sessions = vec![workspace_test_session(
            "photo critique",
            WorkspaceSource::agent(AgentProvider::Codex),
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
            dialogue_results.sessions[0]
                .records
                .iter()
                .map(|record| record.title.as_str())
                .collect::<Vec<_>>(),
            vec!["lighting notes"]
        );
        assert!(content_results.sessions.is_empty());
    }

    #[test]
    fn workspace_search_uses_case_insensitive_regex() {
        let sessions = vec![workspace_test_session(
            "Photo critique",
            WorkspaceSource::agent(AgentProvider::Codex),
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
            WorkspaceSource::agent(AgentProvider::Codex),
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
                WorkspaceSource::agent(AgentProvider::Codex),
                &["needle first", "miss"],
            ),
            workspace_test_session(
                "claude session",
                WorkspaceSource::agent(AgentProvider::Claude),
                &["a1", "needle dialogue"],
            ),
        ];
        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "#needle");

        assert_eq!(output.sessions.len(), 2);
        assert_eq!(output.sessions[0].title, "codex session");
        assert_eq!(output.sessions[0].records[0].title, "needle first");
        assert_eq!(output.sessions[1].title, "claude session");
        assert_eq!(output.sessions[1].records[0].title, "needle dialogue");
        assert_eq!(output.matches.len(), 2);
    }

    #[test]
    fn workspace_search_tracks_match_position_for_navigation() {
        let sessions = vec![WorkspaceSession {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            session_id: "session".to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: "session".to_string(),
            search_title: "session".to_string(),
            records: vec![workspace_test_record(
                WorkspaceSource::agent(AgentProvider::Codex),
                "dialogue",
                "first\nneedle one\nmiddle\nneedle two",
                0,
            )],
            body_loaded: true,
        }];

        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "needle");

        assert_eq!(
            output.matches,
            vec![
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Part {
                        io: WorkPartIo::Input,
                        index: 1,
                    },
                    matched_line: 2,
                },
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Part {
                        io: WorkPartIo::Input,
                        index: 1,
                    },
                    matched_line: 4,
                }
            ]
        );
    }

    #[test]
    fn workspace_search_prefers_hidden_part_targets() {
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "dialogue",
            "visible text",
            0,
        );
        record.parts = vec![sivtr_core::record::WorkPart {
            io: sivtr_core::record::WorkPartIo::Input,
            kind: sivtr_core::record::WorkPartKind::ToolCall,
            index: 1,
            occurred_at: None,
            label: Some("tool".to_string()),
            text: "hidden cargo test".to_string(),
            ansi: None,
        }];
        let sessions = vec![WorkspaceSession {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            session_id: "session".to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: "session".to_string(),
            search_title: "session".to_string(),
            records: vec![record],
            body_loaded: true,
        }];

        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "cargo");

        assert_eq!(
            output.matches,
            vec![WorkspaceSearchMatch {
                session_index: 0,
                dialogue_index: 0,
                at: WorkAt::Part {
                    io: sivtr_core::record::WorkPartIo::Input,
                    index: 1,
                },
                matched_line: 1,
            }]
        );
    }

    #[test]
    fn workspace_search_preserves_line_offsets_inside_part_targets() {
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "dialogue",
            "visible text",
            0,
        );
        record.parts = vec![sivtr_core::record::WorkPart {
            io: sivtr_core::record::WorkPartIo::Output,
            kind: sivtr_core::record::WorkPartKind::ToolOutput,
            index: 1,
            occurred_at: None,
            label: Some("tool".to_string()),
            text: "first line\nneedle one\nmiddle\nneedle two".to_string(),
            ansi: None,
        }];
        let sessions = vec![WorkspaceSession {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            session_id: "session".to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: "session".to_string(),
            search_title: "session".to_string(),
            records: vec![record],
            body_loaded: true,
        }];

        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "needle");

        assert_eq!(
            output.matches,
            vec![
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Part {
                        io: sivtr_core::record::WorkPartIo::Output,
                        index: 1,
                    },
                    matched_line: 2,
                },
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Part {
                        io: sivtr_core::record::WorkPartIo::Output,
                        index: 1,
                    },
                    matched_line: 4,
                },
            ]
        );
        assert_eq!(output.matches[1].content_scroll_index(), 3);
    }

    #[test]
    fn workspace_search_target_ref_round_trips_part_match() {
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "dialogue",
            "visible text",
            0,
        );
        record.parts = vec![sivtr_core::record::WorkPart {
            io: sivtr_core::record::WorkPartIo::Input,
            kind: sivtr_core::record::WorkPartKind::ToolCall,
            index: 1,
            occurred_at: None,
            label: Some("tool".to_string()),
            text: "hidden cargo test".to_string(),
            ansi: None,
        }];
        let sessions = vec![WorkspaceSession {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            session_id: "session".to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: "session".to_string(),
            search_title: "session".to_string(),
            records: vec![record],
            body_loaded: true,
        }];

        let output = WorkspaceSearchIndex::new(&sessions).search(&sessions, "cargo");
        let work_ref =
            workspace_search_target_ref(&output.sessions, &output.matches[0]).expect("work ref");

        assert_eq!(work_ref.to_string(), "codex/test/1/i/1");
    }

    #[test]
    fn clamp_list_state_clears_stale_selection_for_empty_lists() {
        let mut state = ListState::default();
        state.select(Some(0));

        clamp_list_state(&mut state, 0);

        assert_eq!(state.selected(), None);
    }

    #[test]
    fn move_workspace_cursor_up_uses_dialogue_count_for_dialogue_focus() {
        let sessions = vec![workspace_test_session(
            "session",
            WorkspaceSource::agent(AgentProvider::Codex),
            &["dialogue"],
        )];
        let mut source_state = ListState::default();
        source_state.select(Some(0));
        let mut session_state = ListState::default();
        session_state.select(Some(0));
        let mut dialogue_state = ListState::default();
        dialogue_state.select(Some(0));
        let mut selected_dialogues = Vec::new();
        let mut range_anchor = None;
        let mut content_scroll = 0;

        move_workspace_cursor_up(
            WorkspaceFocus::Dialogues,
            &[WorkspaceSource::agent(AgentProvider::Codex)],
            &sessions,
            0,
            &[false],
            &mut source_state,
            &mut session_state,
            &mut dialogue_state,
            &mut selected_dialogues,
            &mut range_anchor,
            &mut content_scroll,
        );

        assert_eq!(dialogue_state.selected(), None);
    }

    #[test]
    fn workspace_picked_content_uses_selected_dialogues_only() {
        let dialogues = vec![
            workspace_test_dialogue("d1", "text 1"),
            workspace_test_dialogue("d2", "text 2"),
            workspace_test_dialogue("d3", "text 3"),
        ];

        let picked = workspace_picked_content(&dialogues, &[false, true, true], 0, None);

        assert_eq!(picked.units.len(), 2);
        assert!(picked.units[0].plain.contains("text 2"));
        assert!(picked.units[1].plain.contains("text 3"));
        assert!(!picked.units[0].plain.contains("text 1"));
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

        let picked = workspace_picked_content(&dialogues, &[false, false], 1, None);

        assert_eq!(picked.units.len(), 1);
        assert!(picked.units[0].plain.contains("text 2"));
        assert!(!picked.units[0].plain.contains("text 1"));
        assert_eq!(picked.selection, CommandSelection::RecentExplicit(vec![1]));
    }

    #[test]
    fn workspace_copy_shortcuts_use_structured_chat_parts_without_headings() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "question".to_string(),
            record: None,
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
        let dialogues = vec![workspace_test_dialogue(
            "question",
            "line 1\nline 2\nline 3",
        )];
        // Override structured copy parts for input shortcut filtering.
        let mut dialogues = dialogues;
        dialogues[0].copy = WorkspaceCopyParts {
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
        };

        let displayed =
            workspace_picked_content_with_line_filter(&dialogues, &[false], 0, Some("2:3"), None)
                .unwrap();
        let input = workspace_picked_content_for_copy_with_line_filter(
            &dialogues,
            &[false],
            0,
            WorkspaceCopyShortcut::Input,
            Some("1,3"),
            None,
            ContentViewMode::Reading,
        )
        .unwrap();

        // Displayed text is Reading-mode render of parts; filter applies to that text.
        assert!(displayed.units[0].plain.lines().count() >= 1);
        assert_eq!(input.units[0].plain, "ask 1\nask 3");
    }

    #[test]
    fn workspace_line_filter_rejects_invalid_specs() {
        let dialogues = vec![workspace_test_dialogue("d1", "alpha\nbeta\ngamma")];

        let err =
            workspace_picked_content_with_line_filter(&dialogues, &[false], 0, Some("x"), None)
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
            source: WorkspaceSource::terminal(),
            work_ref: Some(WorkRef::terminal("shell", 1)),
            title: "cargo test".to_string(),
            record: None,
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
        let dialogue = workspace_test_dialogue("line1", "line1\nline2\nline3\nline4");

        let view = workspace_dialogue_vim_view(&dialogue);
        // Reading mode wraps dialogue with headings/markers — count lines from that render.
        let expected = dialogue.content_text(ContentViewMode::Reading, None);
        assert_eq!(view.raw, expected);
        assert_eq!(view.blocks.len(), 1);
        assert_eq!(view.blocks[0].start, 1);
        assert_eq!(view.blocks[0].end, expected.lines().count().max(1));
        assert_eq!(view.blocks[0].block_text, view.raw);
        assert_eq!(view.blocks[0].input_text, view.raw);
        assert_eq!(view.blocks[0].output_text, view.raw);
    }

    #[test]
    fn workspace_picked_content_prefers_active_part_target_for_display_copy() {
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "dialogue",
            "visible text",
            0,
        );
        record.parts = vec![sivtr_core::record::WorkPart {
            io: sivtr_core::record::WorkPartIo::Input,
            kind: sivtr_core::record::WorkPartKind::ToolCall,
            index: 1,
            occurred_at: None,
            label: Some("tool".to_string()),
            text: "hidden cargo test".to_string(),
            ansi: None,
        }];
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "question".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::from_block(TextPair {
                plain: "visible text".to_string(),
                ansi: String::new(),
            }),
        }];

        let picked = workspace_picked_content(
            &dialogues,
            &[false],
            0,
            Some(WorkAt::Part {
                io: sivtr_core::record::WorkPartIo::Input,
                index: 1,
            }),
        );

        assert_eq!(picked.units[0].plain.trim(), "<:tool:tool call:>");
        // Displayed copy uses Reading mode: fold marker only, no payload.
        assert!(!picked.units[0].plain.contains("hidden cargo test"));
        assert!(!picked.units[0].plain.contains("codex/"));
    }

    fn workspace_test_session(
        title: &str,
        source: WorkspaceSource,
        dialogue_titles: &[&str],
    ) -> WorkspaceSession {
        WorkspaceSession {
            source: source.clone(),
            session_id: title.to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: title.to_string(),
            search_title: title.to_string(),
            records: dialogue_titles
                .iter()
                .enumerate()
                .map(|(idx, dialogue_title)| {
                    workspace_test_record(
                        source.clone(),
                        dialogue_title,
                        &format!("{title}:{dialogue_title}"),
                        idx,
                    )
                })
                .collect(),
            body_loaded: true,
        }
    }

    fn workspace_test_record(
        source: WorkspaceSource,
        title: &str,
        plain: &str,
        index: usize,
    ) -> WorkRecord {
        let (channel, provider, kind) = match source.kind {
            WorkspaceSourceKind::Terminal => {
                (WorkChannel::Terminal, None, WorkRecordKind::TerminalCommand)
            }
            WorkspaceSourceKind::Agent(provider) => (
                WorkChannel::Chat,
                Some(provider.command_name().to_string()),
                WorkRecordKind::ChatTurn,
            ),
        };
        let work_ref = match source.kind {
            WorkspaceSourceKind::Terminal => WorkRef::terminal("test", index + 1),
            WorkspaceSourceKind::Agent(provider) => WorkRef::agent(provider, "test", index + 1),
        };
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: work_ref.clone(),
            source: WorkSource { channel, provider },
            session: WorkSessionRef {
                id: "test".to_string(),
                canonical_id: Some("test-session-0123456789abcdef".to_string()),
                path: None,
            },
            kind,
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: title.to_string(),
            parts: vec![WorkPart {
                io: WorkPartIo::Input,
                kind: WorkPartKind::UserMessage,
                index: 1,
                occurred_at: None,
                label: None,
                text: plain.to_string(),
                ansi: None,
            }],
        }
    }

    fn workspace_test_dialogue(title: &str, plain: &str) -> WorkspaceDialogue {
        let record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            title,
            plain,
            0,
        );
        let pair = crate::commands::browse::text::record_text_to_pair(
            record.copy_text(sivtr_core::record::RecordTextMode::Combined, false),
        );
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            title: title.to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::from_block(pair),
        }
    }
}
