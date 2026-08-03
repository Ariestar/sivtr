use unicode_width::UnicodeWidthChar;

use super::entry::{normalize_newlines, render_input, sanitize_command};

/// Locate the command line in the visible snapshot and return the output that
/// follows it (trailing prompt stripped). `None` means the command line is not
/// in the snapshot — typically because the output scrolled it out of the
/// visible viewport (Windows Terminal exposes only the visible window to Win32
/// reads) — and callers can fall back to recovering the visible output tail.
pub fn extract_output_from_snapshot(
    prompt: &str,
    command: &str,
    snapshot_lines: &[&str],
    width: usize,
) -> Option<String> {
    if snapshot_lines.is_empty() {
        return None;
    }

    let prompt_plain = normalize_newlines(&strip_ansi_escapes::strip_str(prompt));
    let command_plain = sanitize_command(command);
    let expected_input = render_input(&prompt_plain, &command_plain);
    if expected_input.is_empty() {
        return None;
    }

    let expected_lines = normalized_visual_lines(&expected_input, width.max(1));
    if expected_lines.is_empty() {
        return None;
    }
    let prompt_lines = normalized_visual_lines(&prompt_plain, width.max(1));

    let actual_plain_lines: Vec<String> = snapshot_lines
        .iter()
        .map(|line| normalize_visual_line(&strip_ansi_escapes::strip_str(line)))
        .collect();

    // 1) The full rendered input (multi-line prompt + command).
    if let Some(end) = find_last_subsequence_end(&actual_plain_lines, &expected_lines) {
        return Some(finish_output(snapshot_lines, end, &prompt_lines, width));
    }

    // 2) Just the last input line (prompt tail + command); the prompt above it
    //    may have changed since it was rendered (dynamic git/timer status).
    if let Some(last_line) = expected_lines.last() {
        let fallback = [last_line.clone()];
        if let Some(end) = find_last_subsequence_end(&actual_plain_lines, &fallback) {
            return Some(finish_output(snapshot_lines, end, &prompt_lines, width));
        }
    }

    // 3) The command text alone, tolerating an unknown prompt glyph on the same
    //    line (e.g. the first Nushell command, before the prompt cache exists).
    if let Some(end) = find_command_line_end(&actual_plain_lines, &command_plain) {
        return Some(finish_output(snapshot_lines, end, &prompt_lines, width));
    }

    None
}

fn finish_output(
    snapshot_lines: &[&str],
    end: usize,
    prompt_lines: &[String],
    width: usize,
) -> String {
    trim_trailing_prompt(
        snapshot_lines[end..].join("\n").trim_end_matches('\n'),
        prompt_lines,
        width,
    )
}

/// Locate the last snapshot line that carries the command text: first an exact
/// line, then a line that ends with it (a prompt glyph on the same line).
fn find_command_line_end(actual_plain_lines: &[String], command: &str) -> Option<usize> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    if let Some(idx) = actual_plain_lines
        .iter()
        .rposition(|line| line.trim() == command)
    {
        return Some(idx + 1);
    }
    actual_plain_lines
        .iter()
        .rposition(|line| line.trim().ends_with(command))
        .map(|idx| idx + 1)
}

/// Best-effort recovery when the command line has scrolled out of the visible
/// viewport: return the rows that appeared since the previous flush — the
/// still-visible tail of the command's output.
pub fn extract_new_snapshot_rows(previous: &str, current: &str) -> String {
    let old: Vec<String> = previous
        .lines()
        .map(|line| normalize_visual_line(&strip_ansi_escapes::strip_str(line)))
        .collect();
    let raw_new: Vec<&str> = current.lines().collect();
    let plain_new: Vec<String> = raw_new
        .iter()
        .map(|line| normalize_visual_line(&strip_ansi_escapes::strip_str(line)))
        .collect();

    let scroll = scroll_offset(&old, &plain_new);
    let overlap = plain_new.len().min(old.len() - scroll);
    raw_new[overlap..].join("\n")
}

/// How many rows of `old` scrolled off the top before `new` was captured: the
/// smallest offset where `new` starts with a suffix of `old` (least scroll
/// consistent with the data).
fn scroll_offset(old: &[String], new: &[String]) -> usize {
    (0..=old.len())
        .find(|&m| {
            let overlap = new.len().min(old.len() - m);
            overlap == 0 || new[..overlap] == old[m..m + overlap]
        })
        .unwrap_or(old.len())
}

fn find_last_subsequence_end(haystack: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    (0..=haystack.len() - needle.len())
        .rev()
        .find(|&start| haystack[start..start + needle.len()] == *needle)
        .map(|start| start + needle.len())
}

fn normalized_visual_lines(text: &str, width: usize) -> Vec<String> {
    wrap_visual_lines(text, width)
        .into_iter()
        .map(|line| normalize_visual_line(&line))
        .collect()
}

fn trim_trailing_prompt(output: &str, prompt_lines: &[String], width: usize) -> String {
    if output.is_empty() || prompt_lines.is_empty() {
        return output.to_string();
    }

    let mut raw_lines: Vec<&str> = output.lines().collect();
    if raw_lines.is_empty() {
        return String::new();
    }

    let normalized_output = normalized_visual_lines(&strip_ansi_escapes::strip_str(output), width);

    if normalized_output.ends_with(prompt_lines) {
        let keep = raw_lines.len().saturating_sub(prompt_lines.len());
        raw_lines.truncate(keep);
        return raw_lines.join("\n").trim_end_matches('\n').to_string();
    }

    let last_prompt_line = prompt_lines
        .iter()
        .rev()
        .find(|line| !line.is_empty())
        .cloned()
        .unwrap_or_default();
    if last_prompt_line.is_empty() {
        return output.to_string();
    }

    let last_output_line = normalize_visual_line(&strip_ansi_escapes::strip_str(
        raw_lines.last().copied().unwrap_or_default(),
    ));

    if last_output_line == last_prompt_line {
        let keep = raw_lines.len().saturating_sub(prompt_lines.len());
        raw_lines.truncate(keep);
        return raw_lines.join("\n").trim_end_matches('\n').to_string();
    }

    output.to_string()
}

fn normalize_visual_line(line: &str) -> String {
    line.trim_end().to_string()
}

fn wrap_visual_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut wrapped = Vec::new();

    for logical_line in text.lines() {
        if logical_line.is_empty() {
            wrapped.push(String::new());
            continue;
        }

        let mut current = String::new();
        let mut current_width = 0;

        for ch in logical_line.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
            if current_width + ch_width > width && !current.is_empty() {
                wrapped.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }

        wrapped.push(current);
    }

    if text.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
}

#[cfg(test)]
mod tests {
    use super::{extract_new_snapshot_rows, extract_output_from_snapshot, scroll_offset};

    #[test]
    fn extracts_output_after_latest_input_block() {
        let lines = vec![
            "repo on main",
            "❯  cargo test",
            "old",
            "repo on main",
            "❯  cargo test",
            "ok",
        ];
        assert_eq!(
            extract_output_from_snapshot("repo on main\n❯  ", "cargo test", &lines, 120).as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn extracts_output_when_prompt_scrolled_but_command_line_remains() {
        let lines = vec!["❯  cargo test", "line1", "line2"];
        assert_eq!(
            extract_output_from_snapshot("repo on main\n❯  ", "cargo test", &lines, 120).as_deref(),
            Some("line1\nline2")
        );
    }

    #[test]
    fn strips_trailing_prompt_from_captured_output() {
        let lines = vec!["repo on main", "❯  cargo test", "ok", "repo on main", "❯"];
        assert_eq!(
            extract_output_from_snapshot("repo on main\n❯  ", "cargo test", &lines, 120).as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn strips_trailing_prompt_even_when_glyphs_degrade_in_snapshot() {
        let lines = vec![
            "sivtr on main !14 ?2 ⇡1",
            "❯  cargo test",
            "ok",
            "sivtr on main !14 ?2 ⇡1",
            "❯",
        ];
        assert_eq!(
            extract_output_from_snapshot(
                "sivtr on 󰊢 main !14 ?2 ⇡1\n❯  ",
                "cargo test",
                &lines,
                120
            )
            .as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn finds_command_with_unknown_prompt_glyph_prefix() {
        // First command in a Nushell session: the prompt cache is not yet
        // populated, so the expected input is the bare command while the
        // visible line carries the prompt glyph.
        let lines = vec![
            "? flutter devices",
            "Found 2 connected devices:",
            "Windows (desktop)",
        ];
        assert_eq!(
            extract_output_from_snapshot("", "flutter devices", &lines, 120).as_deref(),
            Some("Found 2 connected devices:\nWindows (desktop)")
        );
    }

    #[test]
    fn returns_none_when_command_line_scrolled_away() {
        // Output longer than the visible viewport: the command line is gone.
        let lines = vec![
            "   X Visual Studio is missing necessary components.",
            "Doctor found issues in 3 categories.",
            "PS D:\\Coding\\Sway\\app> ",
        ];
        assert_eq!(
            extract_output_from_snapshot(
                "PS D:\\Coding\\Sway\\app> ",
                "flutter doctor -v",
                &lines,
                120
            ),
            None
        );
    }

    #[test]
    fn returns_empty_output_when_command_line_is_visible() {
        // The anchor is found even though there is no output: `Some("")`, not
        // `None`, so callers do not fall back to the raw snapshot.
        let lines = vec![
            "PS D:\\Coding\\Sway> flutter pub get",
            "PS D:\\Coding\\Sway> ",
        ];
        assert_eq!(
            extract_output_from_snapshot("PS D:\\Coding\\Sway> ", "flutter pub get", &lines, 120)
                .as_deref(),
            Some("")
        );
    }

    #[test]
    fn scroll_offset_finds_least_scroll() {
        let old = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let new = vec![
            "c".to_string(),
            "d".to_string(),
            "x".to_string(),
            "y".to_string(),
        ];
        assert_eq!(scroll_offset(&old, &new), 2);
        assert_eq!(scroll_offset(&old, &old), 0);
    }

    #[test]
    fn extract_new_rows_recovers_rows_since_previous_flush() {
        assert_eq!(
            extract_new_snapshot_rows("a\nb\nc\nd", "c\nd\nX\nY"),
            "X\nY"
        );
        assert_eq!(extract_new_snapshot_rows("a\nb", "X\nY\nZ"), "X\nY\nZ");
        assert_eq!(extract_new_snapshot_rows("a\nb", "a\nb"), "");
    }
}
