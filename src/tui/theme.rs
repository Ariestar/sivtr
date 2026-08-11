//! Shared TUI palette — keep panel chrome and list accents consistent.
//!
//! The active palette lives in a thread-local so the existing accessor call
//! sites (`accent()`, `focus_row()`, …) stay unchanged while the palette is
//! chosen once at TUI startup from the terminal environment and the optional
//! `[theme]` config section.
//!
//! Agent label colors live in a single table, [`provider_colors`]: one row per
//! agent holding all four palette variants. Adding an agent means adding one
//! row there — the compiler enforces the table stays complete because the
//! match is exhaustive over `AgentProvider`.

use ratatui::prelude::{Color, Modifier, Style};
use sivtr_core::ai::AgentProvider;
use sivtr_core::config::ThemeMode;
use std::cell::Cell;

/// Which of the four color schemes is active. Chrome colors and agent label
/// colors are selected from this together, so a single `ACTIVE` cell drives
/// both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    Dark,
    Light,
    Ansi,
    AnsiLight,
}

/// One palette: chrome, text, and selection colors for a color scheme.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) mode: PaletteMode,
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

/// The four agent-label colors for one provider, one per palette. Chosen by
/// hand so light and ANSI terminals get readable, non-RGB variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderColors {
    pub(crate) dark: Color,
    pub(crate) light: Color,
    pub(crate) ansi: Color,
    pub(crate) ansi_light: Color,
}

/// One row per agent: the four curated label colors, selected by the active
/// palette. Exhaustive by construction — adding a provider to `AgentProvider`
/// makes the compiler demand its row here.
pub(crate) const fn provider_colors(provider: AgentProvider) -> ProviderColors {
    match provider {
        AgentProvider::Codex => ProviderColors {
            dark: Color::Rgb(129, 140, 248), // indigo-400
            light: Color::Rgb(79, 70, 229),  // indigo-600
            ansi: Color::Blue,
            ansi_light: Color::Blue,
        },
        AgentProvider::Claude => ProviderColors {
            dark: Color::Rgb(251, 146, 60), // orange-400
            light: Color::Rgb(234, 88, 12), // orange-600
            ansi: Color::Yellow,
            ansi_light: Color::Yellow,
        },
        AgentProvider::Cursor => ProviderColors {
            dark: Color::Rgb(167, 139, 250), // violet-400
            light: Color::Rgb(124, 58, 237), // violet-600
            ansi: Color::Magenta,
            ansi_light: Color::Magenta,
        },
        AgentProvider::OpenCode => ProviderColors {
            dark: Color::Rgb(45, 212, 191),  // teal-400
            light: Color::Rgb(13, 148, 136), // teal-600
            ansi: Color::Cyan,
            ansi_light: Color::Cyan,
        },
        AgentProvider::OpenClaw => ProviderColors {
            dark: Color::Rgb(248, 113, 113), // red-400
            light: Color::Rgb(220, 38, 38),  // red-600
            ansi: Color::Red,
            ansi_light: Color::Red,
        },
        AgentProvider::Hermes => ProviderColors {
            dark: Color::Rgb(250, 204, 21), // yellow-400
            light: Color::Rgb(202, 138, 4), // yellow-600
            ansi: Color::LightYellow,
            ansi_light: Color::Gray,
        },
        AgentProvider::Grok => ProviderColors {
            dark: Color::Rgb(244, 114, 182), // pink-400
            light: Color::Rgb(219, 39, 119), // pink-600
            ansi: Color::LightMagenta,
            ansi_light: Color::DarkGray,
        },
        AgentProvider::Pi => ProviderColors {
            dark: Color::Rgb(74, 222, 128), // green-400
            light: Color::Rgb(22, 163, 74), // green-600
            ansi: Color::Green,
            ansi_light: Color::Green,
        },
        AgentProvider::Qoder => ProviderColors {
            dark: Color::Rgb(34, 211, 238), // cyan-400
            light: Color::Rgb(8, 145, 178), // cyan-600
            ansi: Color::LightCyan,
            ansi_light: Color::Cyan,
        },
        AgentProvider::QoderCn => ProviderColors {
            dark: Color::Rgb(125, 211, 252), // sky-300
            light: Color::Rgb(3, 105, 161),  // sky-600
            ansi: Color::LightCyan,
            ansi_light: Color::Cyan,
        },
        AgentProvider::Gemini => ProviderColors {
            dark: Color::Rgb(96, 165, 250), // blue-400
            light: Color::Rgb(37, 99, 235), // blue-600
            ansi: Color::LightBlue,
            ansi_light: Color::Blue,
        },
        AgentProvider::Goose => ProviderColors {
            dark: Color::Rgb(251, 191, 36), // amber-400
            light: Color::Rgb(217, 119, 6), // amber-600
            ansi: Color::LightYellow,
            ansi_light: Color::Yellow,
        },
        AgentProvider::Qwen => ProviderColors {
            dark: Color::Rgb(232, 121, 249), // fuchsia-400
            light: Color::Rgb(192, 38, 211), // fuchsia-600
            ansi: Color::LightMagenta,
            ansi_light: Color::Magenta,
        },
    }
}

impl Theme {
    /// Dark-background palette (the default).
    pub(crate) const fn dark() -> Self {
        Self {
            mode: PaletteMode::Dark,
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
            mode: PaletteMode::Light,
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
            mode: PaletteMode::Ansi,
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

    /// Terminal-defined palette for light-background terminals without
    /// truecolor: darker ANSI colors that stay readable against a light
    /// default background (the light `Light*` variants used by [`Theme::ansi`]
    /// would wash out).
    pub(crate) const fn ansi_light() -> Self {
        Self {
            mode: PaletteMode::AnsiLight,
            accent: Color::Blue,
            muted: Color::DarkGray,
            dim: Color::Gray,
            local_origin: Color::Green,
            remote_origin: Color::Magenta,
            focus_bg: Color::Blue,
            focus_fg: Color::White,
            selected_bg: Color::DarkGray,
            selected_fg: Color::White,
            range_fg: Color::Yellow,
            title_active: Color::Black,
            muted_text: Color::DarkGray,
            key_hint: Color::Blue,
            footer: Color::DarkGray,
            text_primary: Color::Black,
            failure: Color::Red,
        }
    }
}

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(Theme::dark()) };
}

/// Pick the palette for this process from the config preference. The
/// preference decides light vs dark (auto-detecting the background), and the
/// terminal's truecolor support decides RGB vs ANSI - so a forced light/dark
/// mode still falls back to the ANSI palette when `COLORTERM` is absent,
/// instead of emitting RGB sequences a non-truecolor terminal cannot render.
pub(crate) fn apply(preference: ThemeMode) {
    let light = match preference {
        ThemeMode::Auto => light_background(),
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
    };
    let theme = if supports_truecolor() {
        if light {
            Theme::light()
        } else {
            Theme::dark()
        }
    } else if light {
        // No truecolor does not mean the terminal is dark: pick the darker
        // ANSI palette on a light background so labels stay readable.
        Theme::ansi_light()
    } else {
        Theme::ansi()
    };
    ACTIVE.set(theme);
}

/// Truecolor when the terminal advertises it (`COLORTERM=truecolor|24bit`).
fn supports_truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor") | Ok("24bit")
    )
}

/// Light background when `COLORFGBG`'s background half is a bright ANSI
/// color (8–15); anything else — dark 256-color indexes such as 16 or 232,
/// unset, `default` — stays dark.
fn light_background() -> bool {
    std::env::var("COLORFGBG")
        .ok()
        .is_some_and(|value| colorfgbg_is_light(&value))
}

/// Whether a `COLORFGBG` value ("fg;bg") denotes a light background.
fn colorfgbg_is_light(value: &str) -> bool {
    value
        .rsplit(';')
        .next()
        .and_then(|background| background.parse::<u8>().ok())
        .is_some_and(|background| (8..=15).contains(&background))
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

/// Agent label color for the active palette. Reads the per-provider row from
/// [`provider_colors`] and picks the variant that matches the active scheme.
pub(crate) fn provider_color(provider: AgentProvider) -> Color {
    let colors = provider_colors(provider);
    match ACTIVE.get().mode {
        PaletteMode::Dark => colors.dark,
        PaletteMode::Light => colors.light,
        PaletteMode::Ansi => colors.ansi,
        PaletteMode::AnsiLight => colors.ansi_light,
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

    #[test]
    fn provider_colors_cover_every_agent_with_distinct_schemes() {
        for spec in AgentProvider::all() {
            let colors = provider_colors(spec.provider);
            assert_ne!(
                colors.dark, colors.light,
                "{} must have a light variant distinct from its dark one",
                spec.name
            );
        }
        apply(ThemeMode::Dark);
        let dark = provider_color(AgentProvider::Hermes);
        apply(ThemeMode::Light);
        assert_ne!(provider_color(AgentProvider::Hermes), dark);
    }

    #[test]
    fn ansi_provider_colors_emit_no_rgb() {
        for spec in AgentProvider::all() {
            let colors = provider_colors(spec.provider);
            for color in [colors.ansi, colors.ansi_light] {
                assert!(
                    !matches!(color, Color::Rgb(..)),
                    "{} leaks RGB through the ANSI fallback: {color:?}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn ansi_light_uses_darker_foregrounds_for_light_backgrounds() {
        let dark = Theme::ansi();
        let light = Theme::ansi_light();
        assert_ne!(light.text_primary, dark.text_primary);
        assert_ne!(light.title_active, dark.title_active);
        // The light ANSI palette must avoid the bright foregrounds that wash
        // out against a light default background.
        for color in [
            light.text_primary,
            light.title_active,
            light.footer,
            light.muted_text,
        ] {
            assert!(
                !matches!(
                    color,
                    Color::White
                        | Color::LightRed
                        | Color::LightGreen
                        | Color::LightYellow
                        | Color::LightBlue
                        | Color::LightMagenta
                        | Color::LightCyan
                ),
                "bright foreground on a light ANSI background: {color:?}"
            );
        }
    }

    #[test]
    fn only_bright_ansi_backgrounds_count_as_light() {
        assert!(colorfgbg_is_light("0;15"), "bright white background");
        assert!(!colorfgbg_is_light("15;0"), "black background");
        assert!(!colorfgbg_is_light("15;16"), "256-color dark blue");
        assert!(!colorfgbg_is_light("15;232"), "256-color near-black");
        assert!(!colorfgbg_is_light("15;default"));
        assert!(!colorfgbg_is_light("garbage"));
    }
}
