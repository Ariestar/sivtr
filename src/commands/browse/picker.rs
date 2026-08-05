use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::tui::content::view::{content_link_at, ContentViewMode};
use crate::tui::search::{
    workspace_search_fingerprint, workspace_search_has_query, workspace_search_scope,
    WorkspaceSearchIndex, WorkspaceSearchOutput,
};
use crate::tui::terminal::read_interaction;
use crate::tui::workspace::{
    help_action_for_key, panel_inner_rows, render_workspace, search_match_half, selected_index,
    workspace_help_entries, workspace_hit_test, workspace_layout, ContentIoFocus, ContentIoFrame,
    ContentScrolls, WorkspaceDialogue, WorkspaceFocus, WorkspacePickedContent, WorkspaceSearchView,
    WorkspaceSession, WorkspaceSource, WorkspaceView,
};

use super::content::{
    active_workspace_content_at, handle_line_filter_key, handle_line_filter_paste,
    line_filter_spec, workspace_search_target_ref,
};
use super::help::{apply_workspace_help_action, set_focus, HelpDispatch};
use super::load::{SessionColumn, SessionCtx, SourceLoadState};
use super::nav::{
    clamp_list_state, move_workspace_cursor_down, move_workspace_cursor_up, open_link_target,
    reset_workspace_after_source_change, reset_workspace_dialogue_state,
    resize_workspace_dialogue_selection, row_list_index, source_list_index,
};
use super::panes::{ContentCtx, ContentPane, DialogueCtx, DialoguePane, SourcePane};
use super::selection::{has_selected_sessions, refresh_next_level};
use super::visual::{
    apply_workspace_mouse_scroll, handle_content_mouse_select, handle_visual_select_key,
    scroll_list_state_down, scroll_list_state_up, VisualContentContext, VisualSelectMode,
};
use super::PICK_CANCELLED_MESSAGE;
use crate::pane::{Pane, PaneInput, Viewport};

pub(crate) fn run(
    terminal: &mut crate::tui::terminal::Tui,
    sources: Vec<WorkspaceSource>,
    source_states: Vec<SourceLoadState>,
    selected_sources: Vec<bool>,
    cwd: PathBuf,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    debug_assert_eq!(sources.len(), selected_sources.len());
    debug_assert_eq!(sources.len(), source_states.len());
    let mut selected_sources = selected_sources;
    let mut session_state = ListState::default();
    let mut source_state = ListState::default();
    let mut dialogue_state = ListState::default();
    let mut help_state = ListState::default();
    help_state.select(Some(0));
    let mut focus = initial_focus;
    // Unified Pane stack — each implements crate::pane::Pane.
    // New panes: construct + poll/ensure with PaneInput; no special picker branches.
    let mut source_pane = SourcePane::from_catalog(&sources);
    let mut sessions_pane = SessionColumn::new(sources.clone(), source_states, cwd.clone());
    let mut dialogue_pane = DialoguePane::default();
    let mut content_pane = ContentPane::default();
    let bootstrap = Viewport {
        first: 0,
        visible: 24,
    };
    sessions_pane.kick(&selected_sources, bootstrap, true);
    // Meta-only list — dialogue bodies live in SessionColumn, not here.
    let mut all_sessions = sessions_pane.collect(&selected_sources);
    let mut sessions = all_sessions.clone();
    let mut sessions_dirty = false;
    clamp_list_state(&mut source_state, source_pane.len());
    clamp_list_state(&mut session_state, sessions.len());
    clamp_list_state(&mut dialogue_state, 0);
    let mut selected_sessions = vec![false; sessions.len()];
    let mut selected_dialogues = Vec::new();
    let mut range_anchor = None;
    let mut content_scrolls = ContentScrolls::default();
    let mut content_io_focus = ContentIoFocus::Input;
    let mut content_mode = ContentViewMode::Reading;
    let mut show_help = false;
    let mut show_search = false;
    let mut search_query = String::new();
    let mut search_output = WorkspaceSearchOutput::default();
    let mut search_engine: Option<(WorkspaceSearchIndex, Vec<WorkspaceSession>)> = None;
    let mut search_cursor = 0usize;
    let mut search_dirty = true;
    let mut search_apply_pending = false;
    let mut line_filter_input_open = false;
    let mut line_filter = String::new();
    let mut line_filter_error: Option<String> = None;
    let mut fullscreen = None;
    let mut visual_select_mode = None;
    let mut loading_tick = 0u8;
    // Redraw when an event changed state, a background load landed, or the
    // loading spinner ticked. When idle with no load in flight, block on
    // input instead of redrawing the whole frame at a fixed rate.
    let mut redraw = true;
    // Last-rendered content values, reused by event handlers on iterations
    // that skip redrawing (idle with no state change).
    let mut dialogues: Vec<WorkspaceDialogue> = Vec::new();
    let mut content_frame = ContentIoFrame::default();
    // (engine generation, selected mask, focused index) for the projection in
    // `dialogues`; unchanged redraws reuse it instead of re-cloning bodies.
    let mut dialogues_key: Option<(u64, Vec<bool>, usize)> = None;

    loop {
        // ── Unified pane poll/ensure ───────────────────────────────────────
        let mut reproject = false;
        if sessions_pane.poll() {
            sessions_dirty = true;
            search_dirty = true;
            redraw = true;
        }
        if sessions_dirty {
            all_sessions = sessions_pane.collect(&selected_sources);
            sessions_dirty = false;
            reproject = true;
        }
        if search_dirty {
            if workspace_search_has_query(&search_query) {
                // Rebuild the search corpus and index only when the loaded
                // corpus changed; both are cached across keystrokes so typing
                // does not clone every hydrated session per keypress.
                let fingerprint = workspace_search_fingerprint(
                    &all_sessions,
                    all_sessions
                        .iter()
                        .map(|session| sessions_pane.body_for(session).unwrap_or(&[])),
                );
                if let Some((index, corpus)) = search_engine
                    .as_ref()
                    .filter(|(index, _)| index.fingerprint() == fingerprint)
                {
                    search_output = index.search(corpus, &search_query);
                } else {
                    let corpus: Vec<_> = all_sessions
                        .iter()
                        .map(|s| {
                            let mut full = s.clone();
                            if let Some(recs) = sessions_pane.body_for(s) {
                                full.records = recs.to_vec();
                            }
                            full
                        })
                        .collect();
                    let index = WorkspaceSearchIndex::new(&corpus);
                    search_output = index.search(&corpus, &search_query);
                    search_engine = Some((index, corpus));
                }
            } else {
                search_output = WorkspaceSearchOutput::default();
            }
            if search_cursor >= search_output.matches.len() {
                search_cursor = 0;
            }
            search_apply_pending = true;
            search_dirty = false;
            reproject = true;
        }
        let search_has_query = workspace_search_has_query(&search_query);
        if search_has_query {
            sessions = search_output.sessions.clone();
        } else if reproject {
            sessions = all_sessions.clone();
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
        session_state.select((!sessions.is_empty()).then_some(session_idx));

        let size = terminal.size()?;
        let layout = workspace_layout(
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
            focus,
            fullscreen,
        );

        let _ = source_pane.ensure(
            (),
            &PaneInput::new(
                Viewport::from_panel(source_state.offset(), panel_inner_rows(layout.source)),
                selected_index(&source_state),
            )
            .with_selected(&selected_sources)
            .with_neighbors(1),
        );

        let _ = sessions_pane.ensure(
            SessionCtx {
                selected_sources: &selected_sources,
                sessions: &sessions,
                selected_sessions: &selected_sessions,
                search_active: search_has_query,
            },
            &PaneInput::new(
                Viewport::from_panel(session_state.offset(), panel_inner_rows(layout.sessions)),
                session_idx,
            )
            .with_selected(&selected_sessions)
            .with_neighbors(1),
        );
        // Body hydrate is async — list updates when poll sets sessions_dirty.
        if selected_sessions.len() != sessions.len() {
            selected_sessions.resize(sessions.len(), false);
        }
        let session_idx = selected_index(&session_state).min(sessions.len().saturating_sub(1));
        session_state.select((!sessions.is_empty()).then_some(session_idx));

        let dialogue_focus_hint = pending_match
            .as_ref()
            .map(|matched| matched.dialogue_index)
            .unwrap_or_else(|| selected_index(&dialogue_state));
        if selected_dialogues.len() != dialogue_pane.len() {
            resize_workspace_dialogue_selection(
                dialogue_pane.len(),
                &mut selected_dialogues,
                &mut range_anchor,
            );
        }
        // Body always from SessionColumn — list is meta-only in both browse and search.
        let records = |s: &crate::tui::workspace::WorkspaceSession| sessions_pane.body_for(s);
        dialogue_pane.ensure(
            DialogueCtx {
                sessions: &sessions,
                session_idx,
                selected_sessions: &selected_sessions,
                records: &records,
            },
            &PaneInput::new(
                Viewport::from_panel(dialogue_state.offset(), panel_inner_rows(layout.dialogues)),
                dialogue_focus_hint,
            )
            .with_selected(&selected_dialogues)
            .with_neighbors(1),
        );
        if selected_dialogues.len() != dialogue_pane.len() {
            resize_workspace_dialogue_selection(
                dialogue_pane.len(),
                &mut selected_dialogues,
                &mut range_anchor,
            );
            dialogue_pane.ensure(
                DialogueCtx {
                    sessions: &sessions,
                    session_idx,
                    selected_sessions: &selected_sessions,
                    records: &records,
                },
                &PaneInput::new(
                    Viewport::from_panel(
                        dialogue_state.offset(),
                        panel_inner_rows(layout.dialogues),
                    ),
                    dialogue_focus_hint.min(dialogue_pane.len().saturating_sub(1)),
                )
                .with_selected(&selected_dialogues)
                .with_neighbors(1),
            );
        }

        let dialogue_count = dialogue_pane.len();
        let dialogue_idx = dialogue_focus_hint.min(dialogue_count.saturating_sub(1));
        dialogue_state.select((dialogue_count > 0).then_some(dialogue_idx));
        if pending_match.is_some() {
            range_anchor = None;
        }
        let active_content_at = active_workspace_content_at(
            search_has_query,
            &search_output,
            search_cursor,
            session_idx,
            &selected_dialogues,
            dialogue_idx,
        );

        // Materialize the dialogue projection only when the engine, the
        // selected mask, or the focused row changed. Content scrolling and
        // other-pane activity reuse the last projection instead of cloning
        // dialogue bodies on every redraw.
        let materialize_key = (
            dialogue_pane.generation(),
            selected_dialogues.clone(),
            dialogue_idx,
        );
        if dialogues_key.as_ref() != Some(&materialize_key) {
            dialogue_pane.materialize_into(&selected_dialogues, dialogue_idx, &mut dialogues);
            dialogues_key = Some(materialize_key);
        }

        if redraw {
            redraw = false;
            // List: title borrows. Content/copy: materialize (body only for focus∪select).
            let dialogue_titles: Vec<&str> = dialogue_pane.titles().collect();

            // Resolve where the pending search match lands *before* building the
            // frame: the frame weights geometry toward the focused half, so a
            // focus switch must be visible to the build or the newly active pane
            // keeps the smaller height until the next redraw.
            let pending_half = pending_match.as_ref().map(|matched| {
                let input = dialogues
                    .get(dialogue_idx)
                    .and_then(|dialogue| dialogue.record.as_ref())
                    .and_then(|record| record.part_for_at(matched.at))
                    .is_none_or(|part| part.kind().is_input());
                search_match_half(input, matched.matched_line)
            });
            if let Some((half, _)) = pending_half {
                content_io_focus = half;
            }
            content_frame = ContentIoFrame::build(
                layout.content,
                content_pane.ensure(ContentCtx {
                    dialogues: &dialogues,
                    selected_dialogues: &selected_dialogues,
                    highlighted_idx: dialogue_idx,
                    mode: content_mode,
                    target: active_content_at,
                    area: layout.content,
                    io_focus: content_io_focus,
                }),
                content_mode,
                content_io_focus,
            );
            content_scrolls.clamp_to(content_frame.input_lines, content_frame.output_lines);
            if let Some((half, scroll)) = pending_half {
                let total = content_frame.line_count(half);
                content_scrolls.set(half, scroll.min(total.saturating_sub(1)));
                search_apply_pending = false;
            }

            let source_markers = sessions_pane.markers();
            let body_failures: HashSet<(WorkspaceSource, String)> = sessions
                .iter()
                .filter_map(|s| {
                    sessions_pane
                        .body_failure(s)
                        .map(|_| (s.source.clone(), s.session_id.clone()))
                })
                .collect();
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
                        body_failures,
                        dialogue_titles: &dialogue_titles,
                        dialogues: &dialogues,
                        dialogue_state: &dialogue_state,
                        selected_dialogues: &selected_dialogues,
                        range_anchor,
                        focus,
                        content_scrolls,
                        content_io_focus,
                        content_mode,
                        content_at: active_content_at,
                        show_help,
                        help_state: &help_state,
                        search: (show_search || search_has_query).then_some(WorkspaceSearchView {
                            query: &search_query,
                            scope: workspace_search_scope(&search_query),
                            result_count: sessions.len(),
                            current_match: (!search_output.matches.is_empty())
                                .then_some(search_cursor),
                            match_count: search_output.matches.len(),
                            current_target: search_output
                                .matches
                                .get(search_cursor)
                                .and_then(|matched| {
                                    workspace_search_target_ref(&sessions, matched, &|s| {
                                        sessions_pane.body_for(s)
                                    })
                                })
                                .map(|work_ref| work_ref.to_string()),
                            input_open: show_search,
                        }),
                        line_filter_input_open,
                        line_filter: (!line_filter.is_empty()).then_some(line_filter.as_str()),
                        line_filter_error: line_filter_error.as_deref(),
                        fullscreen,
                        content_selection: visual_select_mode
                            .map(|mode: VisualSelectMode| mode.selection),
                        content_frame: &content_frame,
                    },
                )
            })?;
            // Ratatui reveals the cursor after every frame. Keep it visible
            // only while typing in an overlay or selecting text; hide it
            // otherwise so it does not blink at a stale position during
            // normal browsing.
            if show_search || line_filter_input_open || visual_select_mode.is_some() {
                terminal.show_cursor()?;
            } else {
                terminal.hide_cursor()?;
            }
        }

        // Wait for input. While a load is in flight, keep a short poll so the
        // spinner animates and results repaint without a keypress. When
        // nothing is loading, block until an event arrives instead of
        // redrawing the whole frame at a fixed rate.
        let poll_timeout = if sessions_pane.is_fetching() {
            std::time::Duration::from_millis(100)
        } else {
            std::time::Duration::from_secs(3600)
        };
        if !event::poll(poll_timeout)? {
            if sessions_pane.is_fetching() {
                loading_tick = loading_tick.wrapping_add(1);
                redraw = true;
            }
            continue;
        }
        redraw = true;
        match read_interaction()? {
            Event::Paste(text) => {
                // Bracketed paste delivers the whole clipboard as one event;
                // route it to the open text input (search or line filter).
                if show_search {
                    // Search terms match single lines: fold every line break
                    // into a space so a trailing newline (common when copying
                    // a line) or a multi-line clipboard searches as one query
                    // instead of silently matching nothing.
                    let pasted = normalize_search_paste(&text);
                    let pasted = pasted.trim_end();
                    search_query_edited(
                        |query| query.push_str(pasted),
                        &mut search_query,
                        &mut search_dirty,
                        &mut search_cursor,
                        &mut search_apply_pending,
                        &mut session_state,
                        &mut selected_sessions,
                        &mut dialogue_state,
                        &mut selected_dialogues,
                        &mut range_anchor,
                        &mut content_scrolls,
                    );
                } else if line_filter_input_open {
                    handle_line_filter_paste(&text, &mut line_filter, &mut line_filter_error);
                }
            }
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                // Held keys auto-repeat as Repeat events. Navigation and text
                // input repeat naturally; one-shot toggles (Enter, Esc, v, r,
                // Tab, Space, …) stay press-only so a held key cannot
                // double-fire a commit or toggle.
                if key.kind == KeyEventKind::Repeat
                    && !is_repeat_safe(
                        key.code,
                        key.modifiers,
                        show_search || line_filter_input_open,
                    )
                {
                    continue;
                }

                // Raw mode swallows Ctrl+C; without this the picker is unkillable by terminal
                // control sequences and requires an external kill.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    anyhow::bail!(PICK_CANCELLED_MESSAGE);
                }

                if let Some(mode) = visual_select_mode.as_mut() {
                    let active = content_frame.active(content_io_focus, &mut content_scrolls);
                    if let Some(picked) = handle_visual_select_key(
                        key.code,
                        key.modifiers,
                        mode,
                        active.area,
                        active.text,
                        content_mode,
                        active.scroll,
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
                            reset_workspace_after_source_change(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scrolls,
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
                                &mut content_scrolls,
                                content_io_focus,
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
                                &mut content_scrolls,
                                content_io_focus,
                            );
                        }
                        KeyCode::Backspace => search_query_edited(
                            |query| {
                                query.pop();
                            },
                            &mut search_query,
                            &mut search_dirty,
                            &mut search_cursor,
                            &mut search_apply_pending,
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scrolls,
                        ),
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            search_query_edited(
                                |query| query.clear(),
                                &mut search_query,
                                &mut search_dirty,
                                &mut search_cursor,
                                &mut search_apply_pending,
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scrolls,
                            );
                        }
                        KeyCode::Char(ch) => search_query_edited(
                            |query| query.push(ch),
                            &mut search_query,
                            &mut search_dirty,
                            &mut search_cursor,
                            &mut search_apply_pending,
                            &mut session_state,
                            &mut selected_sessions,
                            &mut dialogue_state,
                            &mut selected_dialogues,
                            &mut range_anchor,
                            &mut content_scrolls,
                        ),
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
                            match apply_workspace_help_action(
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
                                &mut content_scrolls,
                                &mut content_io_focus,
                                &mut content_mode,
                                content_pane.line_count(ContentIoFocus::Input),
                                content_pane.line_count(ContentIoFocus::Output),
                                &mut show_help,
                                &mut show_search,
                                &mut search_query,
                                &mut search_dirty,
                                &mut visual_select_mode,
                                active_content_at,
                                line_filter_spec(&line_filter),
                                &sessions,
                                &dialogues,
                                session_idx,
                                dialogue_idx,
                                dialogue_count,
                                terminal,
                            )? {
                                HelpDispatch::Continue => {}
                                HelpDispatch::Picked(picked) => return Ok(picked),
                                HelpDispatch::Refresh => {
                                    let size = terminal.size()?;
                                    let layout = workspace_layout(
                                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                        focus,
                                        fullscreen,
                                    );
                                    let viewport = Viewport::from_panel(
                                        session_state.offset(),
                                        panel_inner_rows(layout.sessions),
                                    );
                                    refresh_next_level(
                                        focus,
                                        &selected_sources,
                                        &source_state,
                                        &sessions,
                                        &selected_sessions,
                                        &session_state,
                                        &mut sessions_pane,
                                        &mut all_sessions,
                                        &mut search_dirty,
                                        viewport,
                                    );
                                    sessions_dirty = true;
                                }
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

                // Search-result navigation (not in help table — needs match list state).
                if search_has_query && !search_output.matches.is_empty() {
                    match key.code {
                        KeyCode::Char('n') => {
                            search_cursor = (search_cursor + 1) % search_output.matches.len();
                            content_scrolls.clear();
                            search_apply_pending = true;
                            continue;
                        }
                        KeyCode::Char('N') => {
                            search_cursor = search_cursor
                                .checked_sub(1)
                                .unwrap_or_else(|| search_output.matches.len().saturating_sub(1));
                            content_scrolls.clear();
                            search_apply_pending = true;
                            continue;
                        }
                        KeyCode::Esc => {
                            search_query.clear();
                            search_dirty = true;
                            search_cursor = 0;
                            search_apply_pending = false;
                            reset_workspace_after_source_change(
                                &mut session_state,
                                &mut selected_sessions,
                                &mut dialogue_state,
                                &mut selected_dialogues,
                                &mut range_anchor,
                                &mut content_scrolls,
                            );
                            continue;
                        }
                        _ => {}
                    }
                }

                // Table-driven bindings: help registry is the only key declaration.
                if let Some(action) = help_action_for_key(key.code, key.modifiers, focus) {
                    match apply_workspace_help_action(
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
                        &mut content_scrolls,
                        &mut content_io_focus,
                        &mut content_mode,
                        content_pane.line_count(ContentIoFocus::Input),
                        content_pane.line_count(ContentIoFocus::Output),
                        &mut show_help,
                        &mut show_search,
                        &mut search_query,
                        &mut search_dirty,
                        &mut visual_select_mode,
                        active_content_at,
                        line_filter_spec(&line_filter),
                        &sessions,
                        &dialogues,
                        session_idx,
                        dialogue_idx,
                        dialogue_count,
                        terminal,
                    )? {
                        HelpDispatch::Continue => {}
                        HelpDispatch::Picked(picked) => return Ok(picked),
                        HelpDispatch::Refresh => {
                            let size = terminal.size()?;
                            let layout = workspace_layout(
                                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                focus,
                                fullscreen,
                            );
                            let viewport = Viewport::from_panel(
                                session_state.offset(),
                                panel_inner_rows(layout.sessions),
                            );
                            refresh_next_level(
                                focus,
                                &selected_sources,
                                &source_state,
                                &sessions,
                                &selected_sessions,
                                &session_state,
                                &mut sessions_pane,
                                &mut all_sessions,
                                &mut search_dirty,
                                viewport,
                            );
                            sessions_dirty = true;
                        }
                    }
                    continue;
                }

                // Focus number keys (0-3) — derived from WorkspaceFocus, not the help table.
                if let KeyCode::Char(ch) = key.code {
                    if ch.is_ascii_digit() {
                        if let Some(next_focus) =
                            WorkspaceFocus::from_number_key(ch, dialogue_count)
                        {
                            set_focus(&mut focus, &mut fullscreen, next_focus);
                        }
                    }
                }
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
                // Content drag-select (free mouse / Ctrl-block) before list hit-tests.
                {
                    let hit_half = content_frame.areas.hit_test(mouse.column, mouse.row);
                    if let Some(half) = hit_half {
                        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                            content_io_focus = half;
                        }
                        let active = content_frame.active(half, &mut content_scrolls);
                        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                            && visual_select_mode.is_none()
                        {
                            if let Some(target) = content_link_at(
                                active.area,
                                active.text,
                                *active.scroll,
                                content_mode,
                                mouse.column,
                                mouse.row,
                            ) {
                                let _ = open_link_target(&target);
                                continue;
                            }
                        }
                        if handle_content_mouse_select(
                            &mut visual_select_mode,
                            mouse.kind,
                            mouse.modifiers,
                            mouse.column,
                            mouse.row,
                            VisualContentContext {
                                area: active.area,
                                text: active.text,
                                mode: content_mode,
                                scroll: *active.scroll,
                            },
                            true,
                        ) {
                            if visual_select_mode.is_some() {
                                set_focus(&mut focus, &mut fullscreen, WorkspaceFocus::Content);
                            }
                            continue;
                        }
                    } else if visual_select_mode.is_some() {
                        let active = content_frame.active(content_io_focus, &mut content_scrolls);
                        if handle_content_mouse_select(
                            &mut visual_select_mode,
                            mouse.kind,
                            mouse.modifiers,
                            mouse.column,
                            mouse.row,
                            VisualContentContext {
                                area: active.area,
                                text: active.text,
                                mode: content_mode,
                                scroll: *active.scroll,
                            },
                            true,
                        ) {
                            continue;
                        }
                    }
                }
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
                                &mut content_scrolls,
                                content_io_focus,
                            );
                        }
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(clicked_focus) =
                            workspace_hit_test(layout, mouse.column, mouse.row)
                        {
                            // Clicking another pane clears free selection.
                            visual_select_mode = None;
                            set_focus(&mut focus, &mut fullscreen, clicked_focus);
                            match clicked_focus {
                                WorkspaceFocus::Source => {
                                    let vertical = layout.source.height > 3;
                                    if let Some(idx) = source_list_index(
                                        layout.source,
                                        mouse.column,
                                        mouse.row,
                                        &sources,
                                        vertical,
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
                                        content_scrolls.clear();
                                    }
                                }
                                WorkspaceFocus::Dialogues => {
                                    if let Some(idx) =
                                        row_list_index(layout.dialogues, mouse.row, dialogue_count)
                                    {
                                        dialogue_state.select(Some(idx));
                                        content_scrolls.clear();
                                    }
                                }
                                WorkspaceFocus::Content => {}
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

fn normalize_search_paste(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// A search input edit (typed character, backspace, or paste) changed the
/// query: mark the corpus dirty, restart at the first match, and drop the
/// previous result's selection so a shrinking match set cannot leave
/// dangling highlights.
#[allow(clippy::too_many_arguments)]
fn search_query_edited(
    edit: impl FnOnce(&mut String),
    search_query: &mut String,
    search_dirty: &mut bool,
    search_cursor: &mut usize,
    search_apply_pending: &mut bool,
    session_state: &mut ListState,
    selected_sessions: &mut Vec<bool>,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scrolls: &mut ContentScrolls,
) {
    edit(search_query);
    *search_dirty = true;
    *search_cursor = 0;
    *search_apply_pending = true;
    reset_workspace_after_source_change(
        session_state,
        selected_sessions,
        dialogue_state,
        selected_dialogues,
        range_anchor,
        content_scrolls,
    );
}

/// Keys that safely auto-repeat when held. Navigation and scrolling repeat
/// naturally; toggles and one-shot actions (Enter, Esc, v, r, Tab, Space,
/// focus digits, …) stay press-only so a held key cannot double-fire.
/// Inside a text input every character, Backspace, and Delete repeat so
/// holding a key types continuously.
fn is_repeat_safe(code: KeyCode, modifiers: KeyModifiers, text_input: bool) -> bool {
    if text_input
        && matches!(
            code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
        )
    {
        return true;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(code, KeyCode::Char('d') | KeyCode::Char('u'));
    }
    matches!(
        code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Char('j')
            | KeyCode::Char('k')
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Char('n')
            | KeyCode::Char('N')
    )
}

#[cfg(test)]
mod tests {
    fn tool_test_value(text: String) -> serde_json::Value {
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

    use super::super::content::{
        handle_line_filter_key, handle_line_filter_paste, workspace_dialogue_vim_view,
        workspace_picked_content, workspace_picked_content_for_copy,
        workspace_picked_content_for_copy_with_line_filter,
        workspace_picked_content_with_line_filter, workspace_search_target_ref,
        WorkspaceCopyShortcut,
    };
    use super::super::nav::{clamp_list_state, move_workspace_cursor_up};
    use super::super::panes::{DialogueCtx, DialoguePane};
    use crate::commands::select::CommandSelection;
    use crate::pane::{Pane, PaneInput, Viewport};
    use crate::tui::content::view::ContentViewMode;
    use crate::tui::search::{
        workspace_search_fingerprint, workspace_search_query, workspace_search_regex,
        WorkspaceSearchIndex, WorkspaceSearchMatch, WorkspaceSearchScope,
    };
    use crate::tui::workspace::{
        ContentIoFocus, ContentScrolls, TextPair, WorkspaceCopyParts, WorkspaceDialogue,
        WorkspaceFocus, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::widgets::ListState;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{WorkAt, WorkRef};
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkRecord, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
        RECORD_SCHEMA_VERSION,
    };
    use std::time::SystemTime;

    #[test]
    fn repeat_safety_allows_navigation_and_text_input_only() {
        use super::is_repeat_safe;

        // Navigation and scrolling repeat when held.
        assert!(is_repeat_safe(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            false
        ));
        assert!(is_repeat_safe(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            false
        ));
        assert!(is_repeat_safe(KeyCode::Down, KeyModifiers::NONE, false));
        assert!(is_repeat_safe(KeyCode::PageDown, KeyModifiers::NONE, false));
        assert!(is_repeat_safe(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
            false
        ));
        assert!(is_repeat_safe(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
            false
        ));
        assert!(is_repeat_safe(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
            false
        ));

        // Commits and toggles stay press-only.
        assert!(!is_repeat_safe(KeyCode::Enter, KeyModifiers::NONE, false));
        assert!(!is_repeat_safe(KeyCode::Esc, KeyModifiers::NONE, false));
        assert!(!is_repeat_safe(KeyCode::Tab, KeyModifiers::NONE, false));
        assert!(!is_repeat_safe(
            KeyCode::Char('v'),
            KeyModifiers::NONE,
            false
        ));
        assert!(!is_repeat_safe(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
            false
        ));
        assert!(!is_repeat_safe(
            KeyCode::Char(' '),
            KeyModifiers::NONE,
            false
        ));

        // Inside a text input, characters and Backspace type continuously;
        // Enter still commits only once.
        assert!(is_repeat_safe(KeyCode::Char('e'), KeyModifiers::NONE, true));
        assert!(is_repeat_safe(KeyCode::Backspace, KeyModifiers::NONE, true));
        assert!(!is_repeat_safe(KeyCode::Enter, KeyModifiers::NONE, true));
    }

    fn dialogues_for_test(
        sessions: &[WorkspaceSession],
        session_idx: usize,
        selected_sessions: &[bool],
    ) -> Vec<WorkspaceDialogue> {
        let mut pane = DialoguePane::default();
        let records = |s: &WorkspaceSession| {
            sessions
                .iter()
                .find(|x| x.session_id == s.session_id && x.source == s.source)
                .filter(|x| x.body_loaded)
                .map(|x| x.records.as_slice())
        };
        let total: usize = sessions
            .iter()
            .map(|s| s.records.len())
            .sum::<usize>()
            .max(1);
        let vp = Viewport {
            first: 0,
            visible: total.max(40),
        };
        let selected_dialogues = vec![true; total];
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx,
                selected_sessions,
                records: &records,
            },
            &PaneInput::new(vp, 0)
                .with_selected(&selected_dialogues)
                .with_neighbors(total),
        );
        let n = pane.len();
        let selected_dialogues = vec![true; n];
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx,
                selected_sessions,
                records: &records,
            },
            &PaneInput::new(vp, 0)
                .with_selected(&selected_dialogues)
                .with_neighbors(n),
        );
        pane.dialogues()
    }

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

        let dialogues = dialogues_for_test(&sessions, 1, &[false, false]);

        assert_eq!(dialogues.len(), 1);
        assert_eq!(
            dialogues[0].record.as_ref().map(|r| r.title.as_str()),
            Some("o1")
        );
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

        let dialogues = dialogues_for_test(&sessions, 0, &[true, true]);

        assert_eq!(dialogues.len(), 3);
        let titles: Vec<_> = dialogues
            .iter()
            .map(|d| d.record.as_ref().map(|r| r.title.as_str()))
            .collect();
        assert_eq!(titles, [Some("c1"), Some("c2"), Some("a1")]);
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
        // Hit list is meta-only; body stays on the corpus / SessionColumn.
        assert!(output.sessions[0].records.is_empty());
        assert_eq!(output.matches.len(), 1);
        assert_eq!(output.matches[0].dialogue_index, 0);
        assert_eq!(
            sessions[1].records[0]
                .copy_text(sivtr_core::record::RecordTextMode::Combined, false)
                .plain,
            "target session:lighting"
        );
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
        assert!(dialogue_results.sessions[0].records.is_empty());
        assert_eq!(dialogue_results.matches.len(), 1);
        assert_eq!(dialogue_results.matches[0].dialogue_index, 0);
        assert!(content_results.sessions.is_empty());
    }

    #[test]
    fn workspace_search_fingerprint_tracks_searchable_fields() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sessions = vec![workspace_test_session("session", source, &["dialogue"])];
        let fingerprint = workspace_search_fingerprint(
            &sessions,
            sessions.iter().map(|session| session.records.as_slice()),
        );

        let mut session_title_changed = sessions.clone();
        session_title_changed[0].search_title = "renamed session".into();
        assert_ne!(
            fingerprint,
            workspace_search_fingerprint(
                &session_title_changed,
                session_title_changed
                    .iter()
                    .map(|session| session.records.as_slice()),
            )
        );

        let mut dialogue_title_changed = sessions.clone();
        dialogue_title_changed[0].records[0].title = "renamed dialogue".into();
        assert_ne!(
            fingerprint,
            workspace_search_fingerprint(
                &dialogue_title_changed,
                dialogue_title_changed
                    .iter()
                    .map(|session| session.records.as_slice()),
            )
        );

        let mut body_changed = sessions;
        body_changed[0].records[0].parts[0].data = sivtr_core::record::WorkPartData::User {
            content: "changed body".into(),
        };
        assert_ne!(
            fingerprint,
            workspace_search_fingerprint(
                &body_changed,
                body_changed
                    .iter()
                    .map(|session| session.records.as_slice()),
            )
        );
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
        assert_eq!(output.sessions[1].title, "claude session");
        assert!(output.sessions.iter().all(|s| s.records.is_empty()));
        // dialogue_index is the original turn index in the full body.
        assert_eq!(
            output.matches,
            vec![
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Whole,
                    matched_line: 1,
                },
                WorkspaceSearchMatch {
                    session_index: 1,
                    dialogue_index: 1,
                    at: WorkAt::Whole,
                    matched_line: 1,
                },
            ]
        );
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
                    at: WorkAt::Part(1),
                    matched_line: 2,
                },
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Part(1),
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
            seq: 1,
            occurred_at: None,
            data: sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("tool".to_string()),
                input: tool_test_value("hidden cargo test".to_string()),
            },
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
                at: WorkAt::Part(1),
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
            seq: 1,
            occurred_at: None,
            data: sivtr_core::record::WorkPartData::ToolResult {
                call_id: None,
                tool: None,
                output: tool_test_value("first line\nneedle one\nmiddle\nneedle two".to_string()),
            },
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
                    at: WorkAt::Part(1),
                    matched_line: 2,
                },
                WorkspaceSearchMatch {
                    session_index: 0,
                    dialogue_index: 0,
                    at: WorkAt::Part(1),
                    matched_line: 4,
                },
            ]
        );
        assert_eq!(output.matches[1].matched_line.saturating_sub(1), 3);
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
            seq: 1,
            occurred_at: None,
            data: sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("tool".to_string()),
                input: tool_test_value("hidden cargo test".to_string()),
            },
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
        let work_ref = workspace_search_target_ref(&output.sessions, &output.matches[0], &|s| {
            sessions
                .iter()
                .find(|x| x.session_id == s.session_id && x.source == s.source)
                .map(|x| x.records.as_slice())
        })
        .expect("work ref");

        assert_eq!(work_ref.to_string(), "codex/test/1/p1");
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
        let mut content_scrolls = ContentScrolls::default();

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
            &mut content_scrolls,
            ContentIoFocus::Input,
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
    fn line_filter_paste_strips_disallowed_characters() {
        let mut filter = String::new();
        let mut error = Some("previous error".into());

        // Clipboard trailing newline and stray text must not leak into the spec.
        handle_line_filter_paste("1,3:5\n", &mut filter, &mut error);
        assert_eq!(filter, "1,3:5");
        assert!(error.is_none());

        // Fully invalid paste is dropped and the previous error is kept.
        let mut error = Some("previous error".into());
        handle_line_filter_paste("alpha beta\n", &mut filter, &mut error);
        assert_eq!(filter, "1,3:5");
        assert!(error.is_some());
    }

    #[test]
    fn search_paste_normalizes_crlf_as_one_space() {
        assert_eq!(super::normalize_search_paste("foo\r\nbar"), "foo bar");
    }

    #[test]
    fn workspace_command_shortcut_uses_terminal_command_without_prompt() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::terminal(),
            work_ref: Some(WorkRef::terminal("shell", 1)),
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
            seq: 1,
            occurred_at: None,
            data: sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("tool".to_string()),
                input: tool_test_value("hidden cargo test".to_string()),
            },
        }];
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            record: Some(record),
            copy: WorkspaceCopyParts::from_block(TextPair {
                plain: "visible text".to_string(),
                ansi: String::new(),
            }),
        }];

        let picked = workspace_picked_content(&dialogues, &[false], 0, Some(WorkAt::Part(1)));

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
                seq: 1,
                occurred_at: None,
                data: sivtr_core::record::WorkPartData::User {
                    content: plain.to_string(),
                },
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
            record: Some(record),
            copy: WorkspaceCopyParts::from_block(pair),
        }
    }
}
