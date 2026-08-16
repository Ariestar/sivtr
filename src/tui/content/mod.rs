//! Content display: view, markdown, dual-pane IO, part→text.

pub mod block;
pub mod io;
pub mod markdown;
pub mod text;
pub mod tool;
pub mod view;

/// Truncate `text` to at most `max` display characters, appending `…` when
/// the text was cut (a cut line is `max + 1` chars: `max` content plus the
/// ellipsis). The single truncation spelling for every pane.
pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}
