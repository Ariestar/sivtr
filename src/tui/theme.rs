//! Shared TUI palette — keep panel chrome and list accents consistent.
//!
//! The active palette lives in a thread-local so the existing accessor call
//! sites (`accent()`, `focus_row()`, …) stay unchanged while the palette is
//! chosen once at TUI startup from the terminal environment and the optional
//! `[theme]` config section.

use ratatui::prelude::{Color, Modifier, Style};
use sivtr_core::ai::AgentProvider;
use sivtr_core::config::ThemeMode;
use std::cell::Cell;

/// One palette: chrome, text, and selection colors for a color scheme.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) accent: Color,
    pub(crate) muted: Color,
    pub(crate) dim: Color,
    pub(crate) local_origin: Color,
    pub(crate) remote_origin: Color,
    pub(crate) focus_bg: Color,
    pub(crate) focus_fg: Color,
    pub(crate) selected_bg: Color,
    pub(crate) selected_fg: Color,
    pub(crate) range_fg: Color,
    pub(crate) title_active: Color,
    pub(crate) muted_text: Color,
    pub(crate) key_hint: Color,
    pub(crate) footer: Color,
    pub(crate) text_primary: Color,
    pub(crate) failure: Color,
}

impl Theme {
    /// Dark-background palette (the default).
    pub(crate) const fn dark() -> Self {
        Self {
            accent: Color::Rgb(56, 189, 248),         // sky-400
            muted: Color::Rgb(100, 116, 139),         // slate-500
            dim: Color::Rgb(71, 85, 105),             // slate-600
            local_origin: Color::Rgb(52, 211, 153),   // emerald-400
            remote_origin: Color::Rgb(244, 114, 182), // pink-400
            focus_bg: Color::Rgb(30, 64, 175),        // blue-800
            focus_fg: Color::Rgb(240, 249, 255),      // slate-50
            selected_bg: Color::Rgb(51, 65, 85),      // slate-700
            selected_fg: Color::Rgb(226, 232, 240),   // slate-200
            range_fg: Color::Rgb(251, 191, 36),       // amber-400
            title_active: Color::Rgb(224, 242, 254),  // sky-100
            muted_text: Color::Rgb(203, 213, 225),    // slate-300
            key_hint: Color::Rgb(125, 211, 252),      // sky-300
            footer: Color::Rgb(148, 163, 184),        // slate-400
            text_primary: Color::Rgb(226, 232, 240),  // slate-200
            failure: Color::Rgb(248, 113, 113),       // red-400
        }
    }

    /// Light-background palette (darker variants of the same hues).
    pub(crate) const fn light() -> Self {
        Self {
            accent: Color::Rgb(2, 132, 199),         // sky-600
            muted: Color::Rgb(100, 116, 139),        // slate-500
            dim: Color::Rgb(148, 163, 184),          // slate-400
            local_origin: Color::Rgb(5, 150, 105),   // emerald-600
            remote_origin: Color::Rgb(219, 39, 119), // pink-600
            focus_bg: Color::Rgb(219, 234, 254),     // blue-100
            focus_fg: Color::Rgb(30, 41, 59),        // slate-800
            selected_bg: Color::Rgb(226, 232, 240),  // slate-200
            selected_fg: Color::Rgb(51, 65, 85),     // slate-700
            range_fg: Color::Rgb(217, 119, 6),       // amber-600
            title_active: Color::Rgb(15, 23, 42),    // slate-900
            muted_text: Color::Rgb(100, 116, 139),   // slate-500
            key_hint: Color::Rgb(3, 105, 161),       // sky-700
            footer: Color::Rgb(71, 85, 105),         // slate-600
            text_primary: Color::Rgb(30, 41, 59),    // slate-800
            failure: Color::Rgb(220, 38, 38),        // red-600
        }
    }

    /// Terminal-defined palette for terminals without truecolor: ANSI 16
    /// colors render correctly everywhere, while RGB sequences may be
    /// ignored or mis-mapped by older terminals.
    pub(crate) const fn ansi() -> Self {
        Self {
            accent: Color::Cyan,
            muted: Color::DarkGray,
            dim: Color::DarkGray,
            local_origin: Color::Green,
            remote_origin: Color::Magenta,
            focus_bg: Color::Blue,
            focus_fg: Color::White,
            selected_bg: Color::DarkGray,
            selected_fg: Color::White,
            range_fg: Color::Yellow,
            title_active: Color::White,
            muted_text: Color::Gray,
            key_hint: Color::Cyan,
            footer: Color::Gray,
            text_primary: Color::Gray,
            failure: Color::Red,
        }
    }
}

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(Theme::dark()) };
}

/// Pick the palette for this process from the config preference
/// (auto-detect from the environment when not overridden).
pub(crate) fn apply(preference: ThemeMode) {
    let theme = match preference {
        ThemeMode::Auto => detect(),
        ThemeMode::Dark => Theme::dark(),
        ThemeMode::Light => Theme::light(),
    };
    ACTIVE.set(theme);
}

fn detect() -> Theme {
    if supports_truecolor() {
        if light_background() {
            Theme::light()
        } else {
            Theme::dark()
        }
    } else {
        Theme::ansi()
    }
}

/// Truecolor when the terminal advertises it (`COLORTERM=truecolor|24bit`).
fn supports_truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor") | Ok("24bit")
    )
}

/// Light background when `COLORFGBG`'s background half is a bright ANSI
/// color (8–15); anything else (dark, unset, `default`) stays dark.
fn light_background() -> bool {
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|value| value.rsplit(';').next()?.parse::<u8>().ok())
        .is_some_and(|background| background >= 8)
}

/// Active panel chrome (focused border / scrollbar).
pub(crate) fn accent() -> Color {
    ACTIVE.get().accent
}

/// Inactive panel chrome.
pub(crate) fn muted() -> Color {
    ACTIVE.get().muted
}

/// Dim text / empty placeholders.
pub(crate) fn dim() -> Color {
    ACTIVE.get().dim
}

/// Cursor / focus highlight on a list row.
pub(crate) fn focus_row() -> Style {
    let theme = ACTIVE.get();
    Style::default()
        .bg(theme.focus_bg)
        .fg(theme.focus_fg)
        .add_modifier(Modifier::BOLD)
}

/// Multi-selected row (not necessarily focused).
pub(crate) fn selected_row() -> Style {
    let theme = ACTIVE.get();
    Style::default().bg(theme.selected_bg).fg(theme.selected_fg)
}

/// Range selection (visual span).
pub(crate) fn range_row() -> Style {
    Style::default()
        .fg(ACTIVE.get().range_fg)
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
        AgentProvider::Qoder => Color::Rgb(34, 211, 238),  // cyan-400
    }
}

pub(crate) fn terminal_color() -> Color {
    ACTIVE.get().footer
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
    let theme = ACTIVE.get();
    Style::default().fg(if remote {
        theme.remote_origin
    } else {
        theme.local_origin
    })
}

pub(crate) fn title_style(active: bool) -> Style {
    let theme = ACTIVE.get();
    if active {
        Style::default()
            .fg(theme.title_active)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted_text)
    }
}

pub(crate) fn key_hint_style() -> Style {
    Style::default().fg(ACTIVE.get().key_hint)
}

pub(crate) fn footer_style() -> Style {
    Style::default().fg(ACTIVE.get().footer)
}

/// Primary content text (session titles, …).
pub(crate) fn text_primary() -> Color {
    ACTIVE.get().text_primary
}

/// Help / hint descriptions.
pub(crate) fn help_text() -> Color {
    ACTIVE.get().muted_text
}

/// Error / failed-load markers.
pub(crate) fn failure() -> Color {
    ACTIVE.get().failure
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_switches_the_active_palette() {
        apply(ThemeMode::Dark);
        let dark_accent = accent();
        apply(ThemeMode::Light);
        assert_ne!(accent(), dark_accent, "light palette must differ from dark");
        apply(ThemeMode::Dark);
        assert_eq!(accent(), dark_accent);
    }
}
