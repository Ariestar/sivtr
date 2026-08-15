use ratatui::layout::Rect;
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};

use super::theme;

pub(crate) struct Panel {
    key: &'static str,
    name: String,
    active: bool,
    /// Cursor position `(offset, total)` rendered as `(cur/total)` in the
    /// title; `None` hides it (overlays, source strip).
    position: Option<(usize, usize)>,
}

impl Panel {
    pub(crate) fn new(key: &'static str, name: impl Into<String>, active: bool) -> Self {
        Self {
            key,
            name: name.into(),
            active,
            position: None,
        }
    }

    pub(crate) fn with_position(mut self, offset: usize, total: usize) -> Self {
        self.position = Some((offset, total));
        self
    }

    fn title_line(&self) -> Line<'static> {
        let mut spans = if self.key.is_empty() {
            vec![Span::styled(
                self.name.clone(),
                theme::title_style(self.active),
            )]
        } else {
            vec![
                Span::styled(
                    format!(" {} ", self.key),
                    Style::default()
                        .fg(if self.active {
                            theme::accent()
                        } else {
                            theme::muted()
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(self.name.clone(), theme::title_style(self.active)),
            ]
        };
        if let Some((offset, total)) = self.position {
            if total > 0 {
                let current = offset.min(total - 1) + 1;
                spans.push(Span::styled(
                    format!(" ({current}/{total})"),
                    theme::title_style(self.active),
                ));
            }
        }
        Line::from(spans)
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }
}

pub(crate) fn panel_block(panel: &Panel) -> Block<'static> {
    let border = if panel.active() {
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::muted())
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(panel.title_line())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PanelScroll {
    pub(crate) offset: usize,
    pub(crate) total: usize,
    pub(crate) viewport: usize,
}

impl PanelScroll {
    pub(crate) fn new(offset: usize, total: usize, viewport: usize) -> Self {
        Self {
            offset,
            total,
            viewport,
        }
    }
}

pub(crate) fn render_panel_scrollbar(
    frame: &mut Frame,
    area: Rect,
    scroll: PanelScroll,
    active: bool,
) {
    let Some((thumb_top, thumb_height)) =
        panel_scrollbar_thumb(scroll, area.height.saturating_sub(2) as usize)
    else {
        return;
    };
    let x = area.x.saturating_add(area.width).saturating_sub(1);
    let y = area.y.saturating_add(1).saturating_add(thumb_top as u16);
    let style = scrollbar_style(active);
    for row in y..y.saturating_add(thumb_height as u16) {
        if let Some(cell) = frame.buffer_mut().cell_mut((x, row)) {
            cell.set_symbol("┃").set_style(style);
        }
    }
}

fn panel_scrollbar_thumb(scroll: PanelScroll, track_height: usize) -> Option<(usize, usize)> {
    if track_height == 0 || scroll.total == 0 || scroll.viewport == 0 {
        return None;
    }

    let viewport = scroll.viewport.min(scroll.total);
    if scroll.total <= viewport {
        return None;
    }

    let thumb_height = ((viewport * track_height).div_ceil(scroll.total)).clamp(1, track_height);
    let max_offset = scroll.total.saturating_sub(viewport);
    let offset = scroll.offset.min(max_offset);
    let movable = track_height.saturating_sub(thumb_height);
    let numerator = offset * movable + max_offset / 2;
    let thumb_top = numerator.checked_div(max_offset).unwrap_or(0);
    Some((thumb_top, thumb_height))
}

fn scrollbar_style(active: bool) -> Style {
    Style::default()
        .fg(if active {
            theme::accent()
        } else {
            theme::muted()
        })
        .add_modifier(Modifier::BOLD)
}

/// One style entry point for the current row/block in every pane (lists and
/// content). The highlight never depends on the panel being focused: focus is
/// expressed by the border alone, so the current row stays visible after
/// switching panes.
pub(crate) fn active_item_style() -> Style {
    theme::focus_row()
}

/// `●` / `○` marker for selected / unselected rows, used by every pane so the
/// selection dot has one spelling across the whole UI.
pub(crate) fn selection_dot(selected: bool) -> &'static str {
    if selected {
        "●"
    } else {
        "○"
    }
}

pub(crate) fn render_list_panel(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    items: Vec<ListItem<'_>>,
    state: &ListState,
) {
    let list = List::new(items)
        .block(panel_block(&panel))
        .highlight_style(active_item_style())
        .highlight_symbol("");
    let mut local_state = *state;
    frame.render_stateful_widget(list, area, &mut local_state);
}

#[cfg(test)]
mod tests {
    use super::{panel_scrollbar_thumb, PanelScroll};

    #[test]
    fn scrollbar_thumb_scales_with_viewport() {
        assert_eq!(
            panel_scrollbar_thumb(PanelScroll::new(0, 100, 10), 20),
            Some((0, 2))
        );
    }

    #[test]
    fn scrollbar_thumb_tracks_offset() {
        assert_eq!(
            panel_scrollbar_thumb(PanelScroll::new(90, 100, 10), 20),
            Some((18, 2))
        );
    }

    #[test]
    fn scrollbar_is_hidden_when_content_fits() {
        assert_eq!(panel_scrollbar_thumb(PanelScroll::new(0, 5, 10), 20), None);
    }
}
