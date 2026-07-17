//! Shared TUI palette — keep panel chrome and list accents consistent.

use ratatui::prelude::{Color, Modifier, Style};
use sivtr_core::ai::AgentProvider;

/// Active panel chrome (focused border / scrollbar).
pub(crate) fn accent() -> Color {
    Color::Rgb(56, 189, 248) // sky-400
}

/// Inactive panel chrome.
pub(crate) fn muted() -> Color {
    Color::Rgb(100, 116, 139) // slate-500
}

/// Dim text / empty placeholders.
pub(crate) fn dim() -> Color {
    Color::Rgb(71, 85, 105) // slate-600
}

/// Local origin marker.
pub(crate) fn local_origin() -> Color {
    Color::Rgb(52, 211, 153) // emerald-400
}

/// Remote origin marker.
pub(crate) fn remote_origin() -> Color {
    Color::Rgb(244, 114, 182) // pink-400
}

/// Cursor / focus highlight on a list row.
pub(crate) fn focus_row() -> Style {
    Style::default()
        .bg(Color::Rgb(30, 64, 175)) // blue-800
        .fg(Color::Rgb(240, 249, 255)) // slate-50
        .add_modifier(Modifier::BOLD)
}

/// Multi-selected row (not necessarily focused).
pub(crate) fn selected_row() -> Style {
    Style::default()
        .bg(Color::Rgb(51, 65, 85)) // slate-700
        .fg(Color::Rgb(226, 232, 240)) // slate-200
}

/// Range selection (visual span).
pub(crate) fn range_row() -> Style {
    Style::default()
        .fg(Color::Rgb(251, 191, 36)) // amber-400
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn provider_color(provider: AgentProvider) -> Color {
    match provider {
        AgentProvider::Codex => Color::Rgb(129, 140, 248), // indigo-400
        AgentProvider::Claude => Color::Rgb(251, 146, 60), // orange-400
        AgentProvider::Cursor => Color::Rgb(167, 139, 250), // violet-400
        AgentProvider::OpenCode => Color::Rgb(45, 212, 191), // teal-400
        AgentProvider::OpenClaw => Color::Rgb(248, 113, 113), // red-400
        AgentProvider::Hermes => Color::Rgb(250, 204, 21), // yellow-400
        AgentProvider::Grok => Color::Rgb(244, 114, 182),  // pink-400
        AgentProvider::Pi => Color::Rgb(74, 222, 128),     // green-400
    }
}

pub(crate) fn terminal_color() -> Color {
    Color::Rgb(148, 163, 184) // slate-400
}

/// Local `·` / remote `↗` glyph.
pub(crate) fn origin_glyph(remote: bool) -> &'static str {
    if remote {
        "↗"
    } else {
        "·"
    }
}

pub(crate) fn origin_style(remote: bool) -> Style {
    Style::default().fg(if remote {
        remote_origin()
    } else {
        local_origin()
    })
}

pub(crate) fn title_style(active: bool) -> Style {
    if active {
        Style::default()
            .fg(Color::Rgb(224, 242, 254)) // sky-100
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(203, 213, 225)) // slate-300
    }
}

pub(crate) fn key_hint_style() -> Style {
    Style::default().fg(Color::Rgb(125, 211, 252)) // sky-300
}

pub(crate) fn footer_style() -> Style {
    Style::default().fg(Color::Rgb(148, 163, 184)) // slate-400
}
