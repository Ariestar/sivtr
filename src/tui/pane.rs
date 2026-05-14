use ratatui::layout::Rect;
use ratatui::prelude::{Color, Frame, Style};
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
