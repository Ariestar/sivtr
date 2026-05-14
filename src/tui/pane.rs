use ratatui::layout::Rect;
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

pub(crate) struct Panel {
    key: &'static str,
    name: String,
    active: bool,
}

impl Panel {
    pub(crate) fn new(key: &'static str, name: impl Into<String>, active: bool) -> Self {
        Self {
            key,
            name: name.into(),
            active,
        }
    }

    fn title(&self) -> String {
        if self.key.is_empty() {
            return self.name.clone();
        }
        if self.active {
            format!("[{}] {} *", self.key, self.name)
        } else {
            format!("[{}] {}", self.key, self.name)
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }
}

pub(crate) fn panel_block(panel: &Panel) -> Block<'static> {
    let active = panel.active();
    let block = Block::default().borders(Borders::ALL).title(panel.title());
    if active {
        block.border_style(Style::default().fg(Color::Cyan))
    } else {
        block
    }
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

    fn position_label(self) -> Option<String> {
        if self.total == 0 {
            return None;
        }
        Some(format!(
            "[ {}/{} ]",
            self.offset.min(self.total - 1) + 1,
            self.total
        ))
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
        render_panel_scroll_label(frame, area, scroll, active);
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
    render_panel_scroll_label(frame, area, scroll, active);
}

fn render_panel_scroll_label(frame: &mut Frame, area: Rect, scroll: PanelScroll, active: bool) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let Some(label) = scroll.position_label() else {
        return;
    };
    let max_width = area.width.saturating_sub(2) as usize;
    let label = if label.len() > max_width {
        format!("[{}]", scroll.total)
    } else {
        label
    };
    let label_width = label.len() as u16;
    if label_width >= area.width {
        return;
    }
    let x = area
        .x
        .saturating_add(area.width)
        .saturating_sub(1)
        .saturating_sub(label_width);
    let y = area.y.saturating_add(area.height).saturating_sub(1);
    let style = scroll_label_style(active);
    frame.render_widget(Line::styled(label, style), Rect::new(x, y, label_width, 1));
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
    let thumb_top = if max_offset == 0 {
        0
    } else {
        (offset * movable + max_offset / 2) / max_offset
    };
    Some((thumb_top, thumb_height))
}

fn scrollbar_style(active: bool) -> Style {
    let color = if active { Color::Cyan } else { Color::DarkGray };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn scroll_label_style(active: bool) -> Style {
    let color = if active { Color::Cyan } else { Color::DarkGray };
    Style::default().fg(color)
}

pub(crate) fn active_item_style() -> Style {
    Style::default().bg(Color::Blue).fg(Color::White)
}

pub(crate) fn selected_item_style() -> Style {
    Style::default().bg(Color::DarkGray).fg(Color::White)
}

pub(crate) fn inactive_highlight_style() -> Style {
    Style::default()
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
        .highlight_style(if panel.active() {
            active_item_style()
        } else {
            inactive_highlight_style()
        })
        .highlight_symbol("");
    let mut local_state = state.clone();
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
