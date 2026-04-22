use unicode_width::UnicodeWidthChar;

use super::entry::{normalize_newlines, render_input, sanitize_command};

pub fn extract_output_from_snapshot(
    prompt: &str,
    command: &str,
    snapshot_lines: &[&str],
    width: usize,
) -> String {
    if snapshot_lines.is_empty() {
        return String::new();
    }

    let prompt_plain = normalize_newlines(&strip_ansi_escapes::strip_str(prompt));
    let command_plain = sanitize_command(command);
    let expected_input = render_input(&prompt_plain, &command_plain);
    if expected_input.is_empty() {
        return String::new();
    }

    let expected_lines = normalized_visual_lines(&expected_input, width.max(1));
    if expected_lines.is_empty() {
        return String::new();
    }
    let prompt_lines = normalized_visual_lines(&prompt_plain, width.max(1));

    let actual_plain_lines: Vec<String> = snapshot_lines
        .iter()
        .map(|line| normalize_visual_line(&strip_ansi_escapes::strip_str(line)))
        .collect();

    if let Some(end) = find_last_subsequence_end(&actual_plain_lines, &expected_lines) {
        return trim_trailing_prompt(
            snapshot_lines[end..].join("\n").trim_end_matches('\n'),
            &prompt_lines,
            width,
        );
    }

    if let Some(last_line) = expected_lines.last() {
        let fallback = [last_line.clone()];
        if let Some(end) = find_last_subsequence_end(&actual_plain_lines, &fallback) {
            return trim_trailing_prompt(
                snapshot_lines[end..].join("\n").trim_end_matches('\n'),
                &prompt_lines,
                width,
            );
        }
    }

    String::new()
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
    use super::extract_output_from_snapshot;

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
            extract_output_from_snapshot("repo on main\n❯  ", "cargo test", &lines, 120),
            "ok"
        );
    }

    #[test]
    fn extracts_output_when_prompt_scrolled_but_command_line_remains() {
        let lines = vec!["❯  cargo test", "line1", "line2"];
        assert_eq!(
            extract_output_from_snapshot("repo on main\n❯  ", "cargo test", &lines, 120),
            "line1\nline2"
        );
    }

    #[test]
    fn strips_trailing_prompt_from_captured_output() {
        let lines = vec!["repo on main", "❯  cargo test", "ok", "repo on main", "❯"];
        assert_eq!(
            extract_output_from_snapshot("repo on main\n❯  ", "cargo test", &lines, 120),
            "ok"
        );
    }

    #[test]
    fn strips_trailing_prompt_even_when_glyphs_degrade_in_snapshot() {
        let lines = vec![
            "sift on � main !14 ?2 ⇡1",
            "❯  cargo test",
            "ok",
            "sift on � main !14 ?2 ⇡1",
            "❯",
        ];
        assert_eq!(
            extract_output_from_snapshot(
                "sift on 󰊢 main !14 ?2 ⇡1\n❯  ",
                "cargo test",
                &lines,
                120
            ),
            "ok"
        );
    }
}
