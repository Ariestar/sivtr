//! Workspace browser painting (lists, dual content panes, overlays, footer).

use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Position, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, ListItem, ListState, Paragraph};
use regex::Regex;
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

use crate::tui::content::io::ContentIoFocus;
use crate::tui::content::view::{
    content_cursor_position, highlight_spans, render_content_view, ContentSelection, ContentView,
    ContentViewMode,
};
use crate::tui::pane::{
    active_item_style, panel_block, render_list_panel, render_panel_scrollbar, selected_item_style,
    Panel, PanelScroll,
};
use crate::tui::search::{workspace_search_regex_for_query, WorkspaceSearchScope};
use crate::tui::theme;
use crate::tui::workspace::help::{workspace_footer_hotkeys, workspace_help_entries};
use crate::tui::workspace::layout::{selected_index, workspace_layout};
use crate::tui::workspace::model::{
    SourceLoadMarker, WorkspaceDialogue, WorkspaceFocus, WorkspaceFooterView, WorkspaceSearchView,
    WorkspaceSession, WorkspaceSource, WorkspaceView,
};
use sivtr_core::record::{WorkAt, WorkRef};

pub(crate) fn render_workspace(frame: &mut Frame, view: WorkspaceView<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let layout = workspace_layout(area, view.focus, view.fullscreen);

    let dialogue_idx =
        selected_index(view.dialogue_state).min(view.dialogue_titles.len().saturating_sub(1));
    let current_ref = current_content_ref(
        view.dialogues,
        view.selected_dialogues,
        dialogue_idx,
        view.content_at,
    );
    let search_regex = view
        .search
        .as_ref()
        .and_then(|search| workspace_search_regex_for_query(search.query));

    render_source_list(
        frame,
        layout.source,
        view.sources,
        view.selected_sources,
        view.source_markers,
        view.loading_tick,
        view.source_state,
        view.focus == WorkspaceFocus::Source,
    );
    render_session_list(
        frame,
        layout.sessions,
        view.sessions,
        view.selected_sources,
        view.selected_sessions,
        view.session_state,
        &view.body_failures,
        view.search.as_ref(),
        search_regex.as_ref(),
        view.focus == WorkspaceFocus::Sessions,
    );
    render_dialogue_list(
        frame,
        layout.dialogues,
        view.dialogue_titles,
        view.dialogue_state,
        view.selected_sessions,
        view.selected_dialogues,
        view.range_anchor,
        view.search.as_ref(),
        search_regex.as_ref(),
        view.focus == WorkspaceFocus::Dialogues,
    );

    // Dual IO layout + display texts were computed once by the picker for
    // this redraw; reuse them instead of laying the content out a second time.
    let frame_io = view.content_frame;
    let content_active = view.focus == WorkspaceFocus::Content;
    let content_search = view
        .search
        .as_ref()
        .filter(|search| search.scope == WorkspaceSearchScope::Content)
        .and(search_regex.as_ref());
    let title_suffix = content_title_suffix(view.selected_dialogues, current_ref.as_ref());

    for half in [ContentIoFocus::Input, ContentIoFocus::Output] {
        let area = frame_io.areas.area(half);
        if area.height == 0 {
            continue;
        }
        render_content_panel(
            frame,
            area,
            Panel::new(
                WorkspaceFocus::Content.key(),
                format!(
                    "{} ({}){title_suffix}",
                    half.title(),
                    view.content_mode.label()
                ),
                content_active && view.content_io_focus == half,
            ),
            frame_io.texts.display(half),
            view.content_scrolls.get(half),
            view.content_mode,
            content_selection_for_half(view.content_selection, view.content_io_focus, half),
            content_search,
        );
    }

    render_footer(
        frame,
        chunks[1],
        WorkspaceFooterView {
            focus: view.focus,
            show_help: view.show_help,
            search: view.search.as_ref(),
            line_filter_input_open: view.line_filter_input_open,
            line_filter: view.line_filter,
            line_filter_error: view.line_filter_error,
            fullscreen: view.fullscreen,
            content_mode: view.content_mode,
            content_selection: view.content_selection,
            current_ref: current_ref.as_ref(),
        },
    );

    if let Some(selection) = view.content_selection {
        let mut scrolls = view.content_scrolls;
        let active = frame_io.active(view.content_io_focus, &mut scrolls);
        if let Some(pos) = content_cursor_position(
            active.area,
            active.text,
            *active.scroll,
            view.content_mode,
            selection.cursor,
        ) {
            frame.set_cursor_position(pos);
        }
    }

    // Put the caret at the end of the query in whichever overlay is being
    // typed into; the picker keeps the cursor visible only in these modes.
    if let Some(search) = view.search.as_ref().filter(|search| search.input_open) {
        let area = centered_rect(chunks[0], 60, 12);
        position_overlay_cursor(frame, area, search.query);
    } else if view.line_filter_input_open {
        let area = centered_rect(chunks[0], 60, 14);
        position_overlay_cursor(frame, area, view.line_filter.unwrap_or_default());
    }

    if let Some(search) = view.search.filter(|search| search.input_open) {
        render_search_box(frame, centered_rect(chunks[0], 60, 12), search);
    } else if view.line_filter_input_open || view.line_filter_error.is_some() {
        render_line_filter_box(
            frame,
            centered_rect(chunks[0], 60, 14),
            view.line_filter,
            view.line_filter_error,
            view.line_filter_input_open,
        );
    } else if view.show_help {
        render_help_panel(frame, chunks[0], view.help_state);
    }
}

/// Place the cursor just past the typed text inside an overlay's text area.
///
/// Pasted text can contain newlines, and `Paragraph` renders each line on its
/// own row. The caret must therefore sit on the row of the *last* rendered line
/// (clipped to the visible box when the text overflows it), not on the first
/// row with the summed width of every line.
fn position_overlay_cursor(frame: &mut Frame, area: Rect, text: &str) {
    let inner = area.inner(Margin::new(1, 1));
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let caret_row = lines.len().saturating_sub(1).min(inner.height as usize - 1);
    let caret_line = lines.get(caret_row).copied().unwrap_or("");
    let column = UnicodeWidthStr::width(caret_line).min(inner.width as usize) as u16;
    let row = inner.y.saturating_add(caret_row as u16);
    frame.set_cursor_position(Position::new(inner.x.saturating_add(column), row));
}

fn render_footer(frame: &mut Frame, area: Rect, footer: WorkspaceFooterView<'_>) {
    let WorkspaceFooterView {
        focus,
        show_help,
        search,
        line_filter_input_open,
        line_filter,
        line_filter_error,
        fullscreen,
        content_mode,
        content_selection,
        current_ref,
    } = footer;

    let mut spans = if search.is_some() {
        let mut spans = if search.map(|search| search.input_open).unwrap_or(false) {
            footer_control_spans("type search  > session  # dialogue  Enter accept  Esc clear")
        } else {
            footer_control_spans("n next  N previous  Esc clear search  / edit")
        };
        if let Some(label) = search.and_then(search_position_label) {
            spans.extend(footer_status_spans(&label));
        }
        if let Some(target) = search
            .and_then(|search| search.current_target.as_deref())
            .map(|target| format!("match {target}"))
        {
            spans.extend(footer_status_spans(&target));
        }
        spans
    } else {
        let controls = if content_selection.is_some() {
            "select  drag / Ctrl-drag block  y/Enter/Ctrl-c copy  Esc/v clear".to_string()
        } else if show_help {
            "j/k move  Enter execute  Esc/? close help  q cancel".to_string()
        } else {
            workspace_footer_hotkeys(focus)
        };
        footer_control_spans(&controls)
    };

    if fullscreen.is_some() {
        spans.extend(footer_status_spans("fullscreen"));
    }
    if let Some(error) = line_filter_error {
        spans.extend(footer_status_spans(&format!("lines invalid: {error}")));
    } else if line_filter_input_open {
        spans.extend(footer_status_spans(&format!(
            "lines: {}",
            line_filter.unwrap_or_default()
        )));
    } else if let Some(spec) = line_filter.filter(|spec| !spec.is_empty()) {
        spans.extend(footer_status_spans(&format!("lines {spec}")));
    }
    if focus == WorkspaceFocus::Content {
        spans.extend(footer_status_spans(content_mode.label()));
    }
    if matches!(focus, WorkspaceFocus::Dialogues | WorkspaceFocus::Content) {
        if let Some(work_ref) = current_ref {
            spans.extend(footer_status_spans(&format!("ref {work_ref}")));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer_control_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (idx, part) in text
        .split("  ")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if idx > 0 {
            spans.push(Span::styled("  ", theme::footer_style()));
        }
        match part.split_once(' ') {
            Some((key, rest)) => {
                spans.push(Span::styled(
                    key.to_string(),
                    theme::key_hint_style().add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(format!(" {rest}"), theme::footer_style()));
            }
            None => spans.push(Span::styled(
                part.to_string(),
                theme::key_hint_style().add_modifier(Modifier::BOLD),
            )),
        }
    }
    spans
}

fn footer_status_spans(label: &str) -> Vec<Span<'static>> {
    let text = label.trim().trim_start_matches('[').trim_end_matches(']');
    vec![
        Span::styled("  ", theme::footer_style()),
        Span::styled("[", Style::default().fg(theme::muted())),
        Span::styled(
            text.to_string(),
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(theme::muted())),
    ]
}

/// Compact session row: `· cdx  title…` (origin glyph + short badge + title).
///
/// `·` = local, `↗` = remote (named scope on the source or work_ref). A ` [!]`
/// suffix marks a session whose body failed to hydrate.
fn session_row_line(
    choice: &WorkspaceSession,
    selected: bool,
    active_panel: bool,
    base_style: Style,
    highlight: Option<&Regex>,
    body_failed: bool,
) -> Line<'static> {
    let remote = choice.source.is_remote()
        || choice
            .records
            .first()
            .is_some_and(|record| !record.work_ref.is_local());
    let check = if active_panel {
        if selected {
            "● "
        } else {
            "○ "
        }
    } else {
        ""
    };
    let origin = theme::origin_glyph(remote);
    let badge = choice.source.badge();
    let title = compact_session_title(choice);
    // Keep search highlighting over the full visible text, but paint origin/badge
    // with their own colors when the row is not using a solid selection background.
    let plain = format!(
        "{check}{origin} {badge}  {title}{}",
        if body_failed { " [!]" } else { "" }
    );
    if base_style.bg.is_some() {
        return Line::from(highlight_spans(&plain, highlight, base_style));
    }

    let mut spans = Vec::new();
    if !check.is_empty() {
        spans.push(Span::styled(
            check.to_string(),
            Style::default().fg(theme::muted()),
        ));
    }
    spans.push(Span::styled(
        format!("{origin} "),
        theme::origin_style(remote),
    ));
    spans.push(Span::styled(
        format!("{badge}  "),
        Style::default()
            .fg(choice.source.color())
            .add_modifier(Modifier::BOLD),
    ));
    spans.extend(highlight_spans(&title, highlight, Style::default()));
    if body_failed {
        spans.push(Span::styled(" [!]", Style::default().fg(theme::failure())));
    }
    Line::from(spans)
}

fn compact_session_title(choice: &WorkspaceSession) -> String {
    let raw = choice.search_title.trim();
    let raw = if raw.is_empty() {
        choice.title.trim()
    } else {
        raw
    };
    // Strip trailing `  [id]` / ` [N blocks]` noise for the list; full title stays in search_title.
    let without_bracket = raw
        .rsplit_once("  [")
        .map(|(head, _)| head)
        .unwrap_or(raw)
        .trim();
    let without_bracket = without_bracket
        .rsplit_once(" [")
        .filter(|(_, tail)| tail.ends_with(']'))
        .map(|(head, _)| head)
        .unwrap_or(without_bracket)
        .trim();
    truncate_chars(without_bracket, 64)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

fn search_position_label(search: &WorkspaceSearchView<'_>) -> Option<String> {
    let current = search.current_match?;
    Some(format!("{}/{}", current + 1, search.match_count))
}

pub(crate) fn current_content_dialogue<'a>(
    dialogues: &'a [WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> Option<&'a WorkspaceDialogue> {
    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    match selected.as_slice() {
        [] => dialogues.get(highlighted_idx),
        [idx] => dialogues.get(*idx),
        _ => None,
    }
}

pub(crate) fn current_content_ref(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    target: Option<WorkAt>,
) -> Option<WorkRef> {
    current_content_dialogue(dialogues, selected_dialogues, highlighted_idx)
        .and_then(|dialogue| dialogue.content_ref(target))
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn render_search_box(frame: &mut Frame, area: Rect, search: WorkspaceSearchView<'_>) {
    frame.render_widget(Clear, area);
    let title = search_box_title(&search);
    let paragraph =
        Paragraph::new(search_box_body(&search)).block(panel_block(&Panel::new("", title, true)));
    frame.render_widget(paragraph, area);
}

pub(crate) fn search_box_title(search: &WorkspaceSearchView<'_>) -> String {
    let result_label = if search.query.trim().is_empty() {
        "ready".to_string()
    } else if let Some(position) = search_position_label(search) {
        format!("[{position}]")
    } else if search.result_count == 1 {
        "1 result".to_string()
    } else {
        format!("{} results", search.result_count)
    };
    if search.scope == WorkspaceSearchScope::Content {
        format!("Search  ({result_label})")
    } else {
        format!("Search {}  ({})", search.scope.label(), result_label)
    }
}

pub(crate) fn search_box_body(search: &WorkspaceSearchView<'_>) -> String {
    match search.current_target.as_deref() {
        Some(target) => format!("{}\n\nTarget: {target}", search.query),
        None => search.query.to_string(),
    }
}

fn render_line_filter_box(
    frame: &mut Frame,
    area: Rect,
    line_filter: Option<&str>,
    line_filter_error: Option<&str>,
    input_open: bool,
) {
    frame.render_widget(Clear, area);
    let title = if line_filter_error.is_some() {
        "Line Filter  (invalid)".to_string()
    } else if input_open {
        "Line Filter  (editing)".to_string()
    } else {
        "Line Filter".to_string()
    };
    let prompt = line_filter_prompt_text(line_filter, line_filter_error, input_open);
    let paragraph = Paragraph::new(prompt).block(panel_block(&Panel::new(":", title, true)));
    frame.render_widget(paragraph, area);
}

pub(crate) fn line_filter_prompt_text(
    line_filter: Option<&str>,
    line_filter_error: Option<&str>,
    input_open: bool,
) -> String {
    if let Some(error) = line_filter_error {
        return format!(
            "{error}\n\nCurrent: {}\nUse 1-based specs like 2:8 or 1,3,8:12.\nEsc clears the error.",
            line_filter.unwrap_or_default()
        );
    }

    if input_open {
        return format!(
            "{}\n\nEnter keeps displayed lines.\ni/o/y/c copy filtered input, output, block, or command.\nBackspace edits. Esc clears.",
            line_filter.unwrap_or_default()
        );
    }

    format!(
        "{}\n\nUse 1-based specs like 2:8 or 1,3,8:12.",
        line_filter.unwrap_or_default()
    )
}

fn render_help_panel(frame: &mut Frame, area: Rect, state: &ListState) {
    frame.render_widget(Clear, area);
    let items = workspace_help_entries()
        .iter()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<12}", entry.key), theme::key_hint_style()),
                Span::styled(
                    entry.description.to_string(),
                    Style::default().fg(theme::help_text()),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    render_list_panel(frame, area, Panel::new("?", "Help", true), items, state);
    render_list_scrollbar(
        frame,
        area,
        selected_index(state),
        workspace_help_entries().len(),
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_source_list(
    frame: &mut Frame,
    area: Rect,
    sources: &[WorkspaceSource],
    selected_sources: &[bool],
    source_markers: &[SourceLoadMarker],
    loading_tick: u8,
    state: &ListState,
    active: bool,
) {
    let panel = Panel::new(WorkspaceFocus::Source.key(), "Source", active);
    // Compact strip when not focused; vertical list (scrollable) when focused.
    if !active || area.height <= 3 {
        render_source_strip(
            frame,
            area,
            panel,
            sources,
            selected_sources,
            source_markers,
            loading_tick,
            state,
            active,
        );
        return;
    }

    let cursor_idx = selected_index(state).min(sources.len().saturating_sub(1));
    let mut items: Vec<ListItem> = sources
        .iter()
        .enumerate()
        .map(|(idx, source)| {
            let selected = selected_sources.get(idx).copied().unwrap_or(false);
            let focused = idx == cursor_idx;
            let load = source_markers
                .get(idx)
                .copied()
                .unwrap_or(SourceLoadMarker::Idle);
            let marker = load.status_glyph(selected, loading_tick);
            let style = if focused {
                active_item_style()
            } else if selected {
                selected_item_style()
            } else {
                Style::default().fg(source.color())
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(source.label(), style),
            ]))
        })
        .collect();
    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            "<empty>",
            Style::default().fg(theme::dim()),
        )));
    }
    render_list_panel(frame, area, panel, items, state);
    render_list_scrollbar(frame, area, cursor_idx, sources.len(), active);
}

#[allow(clippy::too_many_arguments)]
fn render_source_strip(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    sources: &[WorkspaceSource],
    selected_sources: &[bool],
    source_markers: &[SourceLoadMarker],
    loading_tick: u8,
    state: &ListState,
    active: bool,
) {
    let current = selected_index(state).min(sources.len().saturating_sub(1));
    let mut spans = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("  ", Style::default().fg(theme::dim())));
        }
        let selected = selected_sources.get(idx).copied().unwrap_or(false);
        let focused = idx == current && active;
        let load = source_markers
            .get(idx)
            .copied()
            .unwrap_or(SourceLoadMarker::Idle);
        let marker = load.status_glyph(selected, loading_tick);
        let marker_style = if focused {
            active_item_style()
        } else {
            match load {
                SourceLoadMarker::Failed => Style::default().fg(theme::failure()),
                SourceLoadMarker::Loading => Style::default().fg(theme::accent()),
                SourceLoadMarker::Ready if selected => Style::default().fg(source.color()),
                SourceLoadMarker::Idle if selected => Style::default().fg(theme::muted()),
                _ => Style::default().fg(theme::muted()),
            }
        };
        let label_style = if focused {
            active_item_style()
        } else {
            Style::default()
                .fg(source.color())
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                })
        };
        spans.push(Span::styled(format!("{marker} "), marker_style));
        spans.push(Span::styled(source.label(), label_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled("<empty>", Style::default().fg(theme::dim())));
    }
    let paragraph = Paragraph::new(Line::from(spans)).block(panel_block(&panel));
    frame.render_widget(paragraph, area);
}

#[allow(clippy::too_many_arguments)]
fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    choices: &[WorkspaceSession],
    selected_sources: &[bool],
    selected_sessions: &[bool],
    state: &ListState,
    body_failures: &HashSet<(WorkspaceSource, String)>,
    search: Option<&WorkspaceSearchView<'_>>,
    search_regex: Option<&Regex>,
    active: bool,
) {
    let cursor_idx = selected_index(state);
    let has_selection = selected_sessions.iter().any(|selected| *selected);
    let mut items: Vec<ListItem> = choices
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let selected = selected_sessions.get(idx).copied().unwrap_or(false);
            let focused = active && !has_selection && idx == cursor_idx;
            let base_style = if selected {
                selected_item_style()
            } else if focused {
                active_item_style()
            } else {
                Style::default()
            };
            let highlight = search
                .filter(|search| search.scope == WorkspaceSearchScope::Session)
                .and(search_regex);
            ListItem::new(session_row_line(
                choice,
                selected,
                active,
                base_style,
                highlight,
                body_failures.contains(&(choice.source.clone(), choice.session_id.clone())),
            ))
        })
        .collect();
    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            "<empty>",
            Style::default().fg(theme::dim()),
        )));
    }
    render_list_panel(
        frame,
        area,
        Panel::new(
            WorkspaceFocus::Sessions.key(),
            selected_parent_title("Sessions", selected_sources, "source", "sources"),
            active,
        ),
        items,
        state,
    );
    render_list_scrollbar(frame, area, cursor_idx, choices.len(), active);
}

#[allow(clippy::too_many_arguments)]
fn render_dialogue_list(
    frame: &mut Frame,
    area: Rect,
    titles: &[&str],
    state: &ListState,
    selected_sessions: &[bool],
    selected_dialogues: &[bool],
    range_anchor: Option<usize>,
    search: Option<&WorkspaceSearchView<'_>>,
    search_regex: Option<&Regex>,
    active: bool,
) {
    let highlighted_idx = selected_index(state);
    let has_selection = selected_dialogues.iter().any(|selected| *selected);
    let mut items: Vec<ListItem> = titles
        .iter()
        .enumerate()
        .map(|(idx, title)| {
            let in_range = range_anchor
                .map(|anchor| {
                    idx >= anchor.min(highlighted_idx) && idx <= anchor.max(highlighted_idx)
                })
                .unwrap_or(false);
            let selected = selected_dialogues.get(idx).copied().unwrap_or(false);
            let marker = if active {
                if selected {
                    "● "
                } else {
                    "○ "
                }
            } else {
                ""
            };
            let line = format!("{marker}{title}");
            let highlight = search
                .filter(|search| search.scope == WorkspaceSearchScope::Dialogue)
                .and(search_regex);
            if in_range {
                ListItem::new(Line::from(Span::styled(line, theme::range_row())))
            } else if selected {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    selected_item_style(),
                )))
            } else if !has_selection && idx == highlighted_idx {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    active_item_style(),
                )))
            } else {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    Style::default(),
                )))
            }
        })
        .collect();

    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            "<empty>",
            Style::default().fg(theme::dim()),
        )));
    }

    render_list_panel(
        frame,
        area,
        Panel::new(
            WorkspaceFocus::Dialogues.key(),
            selected_parent_title("Dialogues", selected_sessions, "session", "sessions"),
            active,
        ),
        items,
        state,
    );
    render_list_scrollbar(frame, area, highlighted_idx, titles.len(), active);
}

fn render_list_scrollbar(
    frame: &mut Frame,
    area: Rect,
    selected_idx: usize,
    total: usize,
    active: bool,
) {
    render_panel_scrollbar(
        frame,
        area,
        PanelScroll::new(selected_idx, total, panel_viewport_height(area)),
        active,
    );
}

fn panel_viewport_height(area: Rect) -> usize {
    area.height.saturating_sub(2) as usize
}

/// Inner row count for a panel rect (borders already accounted).
pub(crate) fn panel_inner_rows(area: Rect) -> usize {
    panel_viewport_height(area).max(1)
}

#[allow(clippy::too_many_arguments)]
fn render_content_panel(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    selection: Option<ContentSelection>,
    search_regex: Option<&Regex>,
) {
    render_content_view(
        frame,
        area,
        panel,
        ContentView {
            text,
            scroll,
            search_regex,
            mode,
            selection,
        },
    );
}

fn content_selection_for_half(
    selection: Option<ContentSelection>,
    active: ContentIoFocus,
    half: ContentIoFocus,
) -> Option<ContentSelection> {
    selection.filter(|_| active == half)
}

fn selected_parent_title(
    title: &str,
    selected_parent_items: &[bool],
    singular: &str,
    plural: &str,
) -> String {
    let count = selected_parent_items
        .iter()
        .filter(|selected| **selected)
        .count();
    if count == 0 {
        title.to_string()
    } else if count == 1 {
        format!("{title}: 1 {singular} selected")
    } else {
        format!("{title}: {count} {plural} selected")
    }
}

fn content_title_suffix(selected_dialogues: &[bool], current_ref: Option<&WorkRef>) -> String {
    let count = selected_dialogues.iter().filter(|s| **s).count();
    let select = match count {
        0 => String::new(),
        1 => ": 1 dialogue selected".to_string(),
        n => format!(": {n} dialogues selected"),
    };
    match (select.as_str(), current_ref) {
        ("", None) => String::new(),
        ("", Some(r)) => format!(" [{r}]"),
        (s, None) => s.to_string(),
        (s, Some(r)) => format!("{s} [{r}]"),
    }
}

#[cfg(test)]
pub(crate) fn content_title(
    mode: ContentViewMode,
    selected_dialogues: &[bool],
    current_ref: Option<&WorkRef>,
) -> String {
    format!(
        "Content ({}){}",
        mode.label(),
        content_title_suffix(selected_dialogues, current_ref)
    )
}

#[cfg(test)]
mod tests {
    use super::position_overlay_cursor;
    use ratatui::backend::{Backend, TestBackend};
    use ratatui::layout::Rect;
    use ratatui::prelude::Terminal;

    /// Render the overlay at `(2, 1, 14, 4)` — inner text area `(3, 2, 12, 2)` —
    /// and return the caret position the frame recorded.
    fn cursor_for(text: &str) -> (u16, u16) {
        let backend = TestBackend::new(20, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                position_overlay_cursor(frame, Rect::new(2, 1, 14, 4), text);
            })
            .unwrap();
        let pos = terminal.backend_mut().get_cursor_position().unwrap();
        (pos.x, pos.y)
    }

    #[test]
    fn overlay_cursor_follows_single_line_text() {
        assert_eq!(cursor_for("2:3"), (6, 2));
        assert_eq!(cursor_for(""), (3, 2));
    }

    #[test]
    fn overlay_cursor_tracks_the_last_rendered_line() {
        // "1,3\n5:7" renders "1,3" on the first row and "5:7" on the second;
        // the caret must sit at the end of "5:7", not at the summed width on
        // the first row.
        assert_eq!(cursor_for("1,3\n5:7"), (6, 3));
        // A trailing newline opens an empty final row for the caret.
        assert_eq!(cursor_for("1,3\n"), (3, 3));
    }

    #[test]
    fn overlay_cursor_clamps_to_the_visible_box() {
        // More lines than the box has rows: the caret stays on the last visible
        // row, at the end of the line rendered there.
        assert_eq!(cursor_for("1\n2\n3\n4"), (4, 3));
        // A line wider than the box clamps the column to the right edge.
        assert_eq!(cursor_for("1\n0123456789abcdefghij"), (15, 3));
    }

    #[test]
    fn session_row_body_uses_default_foreground() {
        use super::session_row_line;
        use crate::tui::workspace::model::{WorkspaceSession, WorkspaceSource};
        use ratatui::prelude::Style;
        use sivtr_core::ai::AgentProvider;
        use std::time::SystemTime;

        let session = WorkspaceSession {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            session_id: "abc123".to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: "title".to_string(),
            search_title: "title".to_string(),
            records: Vec::new(),
            body_loaded: false,
        };
        let line = session_row_line(&session, false, true, Style::default(), None, false);
        // Non-selected session rows paint body text with the terminal default
        // foreground, matching dialogue rows and the content pane.
        let title_span = line
            .spans
            .iter()
            .find(|span| span.content == "title")
            .expect("title span");
        assert_eq!(title_span.style.fg, None);
    }
}
