//! Four-column workspace geometry and hit-testing.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::ListState;

use crate::tui::workspace::model::WorkspaceFocus;

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceLayout {
    pub(crate) source: Rect,
    pub(crate) sessions: Rect,
    pub(crate) dialogues: Rect,
    pub(crate) content: Rect,
}

pub(crate) fn selected_index(state: &ListState) -> usize {
    state.selected().unwrap_or(0)
}

pub(crate) fn can_open_dialogue_vim(focus: WorkspaceFocus, dialogue_count: usize) -> bool {
    dialogue_count > 0
        && matches!(
            focus,
            WorkspaceFocus::Sessions | WorkspaceFocus::Dialogues | WorkspaceFocus::Content
        )
}

pub(crate) fn workspace_layout(
    area: Rect,
    focus: WorkspaceFocus,
    fullscreen: Option<WorkspaceFocus>,
) -> WorkspaceLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    if let Some(fullscreen) = fullscreen {
        return match fullscreen {
            WorkspaceFocus::Source => WorkspaceLayout {
                source: chunks[0],
                sessions: Rect::default(),
                dialogues: Rect::default(),
                content: Rect::default(),
            },
            WorkspaceFocus::Sessions => WorkspaceLayout {
                source: Rect::default(),
                sessions: chunks[0],
                dialogues: Rect::default(),
                content: Rect::default(),
            },
            WorkspaceFocus::Dialogues => WorkspaceLayout {
                source: Rect::default(),
                sessions: Rect::default(),
                dialogues: chunks[0],
                content: Rect::default(),
            },
            WorkspaceFocus::Content => WorkspaceLayout {
                source: Rect::default(),
                sessions: Rect::default(),
                dialogues: Rect::default(),
                content: chunks[0],
            },
        };
    }

    let constraints = match focus {
        WorkspaceFocus::Source | WorkspaceFocus::Sessions => [
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ],
        WorkspaceFocus::Dialogues => [
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ],
        WorkspaceFocus::Content => [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(50),
        ],
    };
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(chunks[0]);
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(main_chunks[0]);

    WorkspaceLayout {
        source: left_chunks[0],
        sessions: left_chunks[1],
        dialogues: main_chunks[1],
        content: main_chunks[2],
    }
}

pub(crate) fn workspace_hit_test(
    layout: WorkspaceLayout,
    column: u16,
    row: u16,
) -> Option<WorkspaceFocus> {
    if rect_contains(layout.source, column, row) {
        Some(WorkspaceFocus::Source)
    } else if rect_contains(layout.sessions, column, row) {
        Some(WorkspaceFocus::Sessions)
    } else if rect_contains(layout.dialogues, column, row) {
        Some(WorkspaceFocus::Dialogues)
    } else if rect_contains(layout.content, column, row) {
        Some(WorkspaceFocus::Content)
    } else {
        None
    }
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

