use crate::buffer::line::{AnsiColor, StyledSpan};

/// Strip all ANSI escape sequences from the input string, returning plain text.
pub fn strip_ansi(input: &str) -> String {
    let bytes = strip_ansi_escapes::strip(input);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Current style state while parsing.
#[derive(Clone, Default)]
struct StyleState {
    fg: Option<AnsiColor>,
    bg: Option<AnsiColor>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

impl StyleState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn is_default(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && !self.bold
            && !self.dim
            && !self.italic
            && !self.underline
    }

    fn to_span(&self, start: usize, end: usize) -> StyledSpan {
        StyledSpan {
            start,
            end,
            fg: self.fg.clone(),
            bg: self.bg.clone(),
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            dim: self.dim,
        }
    }
}

/// Map standard ANSI color code (30-37, 90-97 for fg; 40-47, 100-107 for bg)
/// to an indexed color value.
fn sgr_to_indexed_color(code: u16) -> Option<u8> {
    match code {
        30..=37 => Some((code - 30) as u8),
        40..=47 => Some((code - 40) as u8),
        90..=97 => Some((code - 90 + 8) as u8),
        100..=107 => Some((code - 100 + 8) as u8),
        _ => None,
    }
}

/// Parse ANSI escape sequences and produce styled spans.
///
/// Each span maps a byte range in the *cleaned* text to a style.
/// Walks through the raw input, tracks style state via SGR codes,
/// and emits spans whenever the style changes.
pub fn parse_styles(raw_line: &str) -> Vec<StyledSpan> {
    let mut spans: Vec<StyledSpan> = Vec::new();
    let mut state = StyleState::default();
    let mut clean_offset: usize = 0; // byte offset in cleaned text
    let mut span_start: usize = 0;

    let bytes = raw_line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'[' {
            // Found ESC[ 鈥?parse CSI sequence
            let _seq_start = i;
            i += 2; // skip ESC[

            // Collect parameter bytes (digits and semicolons)
            let param_start = i;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                i += 1;
            }

            // Final byte determines the command
            if i < len {
                let final_byte = bytes[i];
                i += 1; // consume final byte

                if final_byte == b'm' {
                    // SGR sequence 鈥?flush current span and apply new style
                    if clean_offset > span_start {
                        spans.push(state.to_span(span_start, clean_offset));
                        span_start = clean_offset;
                    }

                    let params_str =
                        std::str::from_utf8(&bytes[param_start..i - 1]).unwrap_or("");
                    apply_sgr(&mut state, params_str);
                }
                // Ignore non-SGR CSI sequences
            }
        } else {
            // Regular character 鈥?count its byte length in the clean output
            let _ch_start = i;
            // Decode one UTF-8 character
            if let Some(ch) = raw_line[i..].chars().next() {
                let ch_len = ch.len_utf8();
                clean_offset += ch_len;
                i += ch_len;
            } else {
                i += 1;
                clean_offset += 1;
            }
        }
    }

    // Flush final span
    if clean_offset > span_start {
        spans.push(state.to_span(span_start, clean_offset));
    }

    spans
}

/// Apply SGR (Select Graphic Rendition) parameters to the style state.
fn apply_sgr(state: &mut StyleState, params: &str) {
    if params.is_empty() {
        state.reset();
        return;
    }

    let codes: Vec<u16> = params
        .split(';')
        .filter_map(|s| s.parse::<u16>().ok())
        .collect();

    let mut ci = 0;
    while ci < codes.len() {
        match codes[ci] {
            0 => state.reset(),
            1 => state.bold = true,
            2 => state.dim = true,
            3 => state.italic = true,
            4 => state.underline = true,
            22 => {
                state.bold = false;
                state.dim = false;
            }
            23 => state.italic = false,
            24 => state.underline = false,
            // Standard foreground colors
            30..=37 | 90..=97 => {
                state.fg = sgr_to_indexed_color(codes[ci]).map(AnsiColor::Indexed);
            }
            39 => state.fg = None, // default fg
            // Standard background colors
            40..=47 | 100..=107 => {
                state.bg = sgr_to_indexed_color(codes[ci]).map(AnsiColor::Indexed);
            }
            49 => state.bg = None, // default bg
            // Extended color: 38;5;N (256-color) or 38;2;R;G;B (truecolor)
            38 => {
                if ci + 1 < codes.len() {
                    match codes[ci + 1] {
                        5 if ci + 2 < codes.len() => {
                            state.fg = Some(AnsiColor::Indexed(codes[ci + 2] as u8));
                            ci += 2;
                        }
                        2 if ci + 4 < codes.len() => {
                            state.fg = Some(AnsiColor::Rgb(
                                codes[ci + 2] as u8,
                                codes[ci + 3] as u8,
                                codes[ci + 4] as u8,
                            ));
                            ci += 4;
                        }
                        _ => {}
                    }
                }
            }
            // Extended background: 48;5;N or 48;2;R;G;B
            48 => {
                if ci + 1 < codes.len() {
                    match codes[ci + 1] {
                        5 if ci + 2 < codes.len() => {
                            state.bg = Some(AnsiColor::Indexed(codes[ci + 2] as u8));
                            ci += 2;
                        }
                        2 if ci + 4 < codes.len() => {
                            state.bg = Some(AnsiColor::Rgb(
                                codes[ci + 2] as u8,
                                codes[ci + 3] as u8,
                                codes[ci + 4] as u8,
                            ));
                            ci += 4;
                        }
                        _ => {}
                    }
                }
            }
            _ => {} // ignore unknown codes
        }
        ci += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_plain_text() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn test_strip_colored_text() {
        assert_eq!(strip_ansi("\x1b[31mhello\x1b[0m"), "hello");
    }

    #[test]
    fn test_strip_complex_escapes() {
        let input = "\x1b[1;32m鉁揬x1b[0m \x1b[90mtest passed\x1b[0m";
        assert_eq!(strip_ansi(input), "鉁?test passed");
    }

    #[test]
    fn test_strip_empty() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_parse_styles_plain() {
        let spans = parse_styles("hello");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 5);
        assert_eq!(spans[0].fg, None);
    }

    #[test]
    fn test_parse_styles_empty() {
        let spans = parse_styles("");
        assert!(spans.is_empty());
    }

    #[test]
    fn test_parse_red_text() {
        // \x1b[31mhello\x1b[0m
        let spans = parse_styles("\x1b[31mhello\x1b[0m");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].fg, Some(AnsiColor::Indexed(1))); // red
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 5);
    }

    #[test]
    fn test_parse_bold_green() {
        // \x1b[1;32m鉁揬x1b[0m world
        let spans = parse_styles("\x1b[1;32m鉁揬x1b[0m world");
        assert_eq!(spans.len(), 2);
        // First span: bold green "鉁?
        assert_eq!(spans[0].fg, Some(AnsiColor::Indexed(2)));
        assert!(spans[0].bold);
        // Second span: reset " world"
        assert_eq!(spans[1].fg, None);
        assert!(!spans[1].bold);
    }

    #[test]
    fn test_parse_256_color() {
        // \x1b[38;5;208mtext\x1b[0m
        let spans = parse_styles("\x1b[38;5;208mtext\x1b[0m");
        assert_eq!(spans[0].fg, Some(AnsiColor::Indexed(208)));
    }

    #[test]
    fn test_parse_rgb_color() {
        // \x1b[38;2;255;128;0mtext\x1b[0m
        let spans = parse_styles("\x1b[38;2;255;128;0mtext\x1b[0m");
        assert_eq!(spans[0].fg, Some(AnsiColor::Rgb(255, 128, 0)));
    }
}
