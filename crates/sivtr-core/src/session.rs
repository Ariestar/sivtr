use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub prompt: String,
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub last_command_id: Option<String>,
    pub last_command: Option<String>,
}

pub fn load_entries(path: &Path) -> Result<Vec<SessionEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file =
        fs::File::open(path).with_context(|| format!("Failed to read session log: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!("Failed to read session log line {}: {}", idx + 1, path.display())
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: SessionEntry = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse session log line {} as structured entry: {}",
                idx + 1,
                path.display()
            )
        })?;
        entries.push(sanitize_entry(entry));
    }

    Ok(entries)
}

pub fn append_entry(path: &Path, entry: &SessionEntry) -> Result<()> {
    reset_invalid_log_if_needed(path)?;
    rewrite_sanitized_log_if_needed(path)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let entry = sanitize_entry(entry.clone());
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open session log for append: {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&entry).context("Failed to encode session entry")?)?;
    Ok(())
}

pub fn load_state(path: &Path) -> Result<SessionState> {
    if !path.exists() {
        return Ok(SessionState::default());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read session state: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse session state: {}", path.display()))
}

pub fn save_state(path: &Path, state: &SessionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string(state).context("Failed to encode session state")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write session state: {}", path.display()))
}

pub fn render_input(prompt: &str, command: &str) -> String {
    let prompt = sanitize_prompt(prompt);
    let command = sanitize_command(command);

    if prompt.is_empty() {
        return command;
    }
    if command.is_empty() {
        return prompt;
    }

    let mut lines: Vec<String> = prompt.lines().map(str::to_string).collect();
    if lines.is_empty() {
        return command;
    }

    if prompt.ends_with('\n') {
        lines.push(command);
    } else if let Some(last) = lines.last_mut() {
        last.push_str(&command);
    }

    lines.join("\n")
}

pub fn render_entry(entry: &SessionEntry) -> String {
    let input = render_input(&entry.prompt, &entry.command);
    let output = sanitize_output(&entry.output);

    match (input.is_empty(), output.is_empty()) {
        (false, false) => format!("{input}\n{output}"),
        (false, true) => input,
        (true, false) => output,
        (true, true) => String::new(),
    }
}

pub fn render_entries(entries: &[SessionEntry]) -> String {
    entries
        .iter()
        .map(render_entry)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

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
            snapshot_lines[end..]
                .join("\n")
                .trim_end_matches('\n'),
            &prompt_lines,
            width,
        );
    }

    if let Some(last_line) = expected_lines.last() {
        let fallback = [last_line.clone()];
        if let Some(end) = find_last_subsequence_end(&actual_plain_lines, &fallback) {
            return trim_trailing_prompt(
                snapshot_lines[end..]
                    .join("\n")
                    .trim_end_matches('\n'),
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

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end_matches('\n').to_string()
}

fn sanitize_prompt(prompt: &str) -> String {
    normalize_newlines(&strip_ansi_escapes::strip_str(prompt))
}

fn sanitize_command(command: &str) -> String {
    normalize_newlines(command)
}

fn sanitize_output(output: &str) -> String {
    normalize_newlines(&strip_ansi_escapes::strip_str(output))
}

fn sanitize_entry(entry: SessionEntry) -> SessionEntry {
    SessionEntry {
        prompt: sanitize_prompt(&entry.prompt),
        command: sanitize_command(&entry.command),
        output: sanitize_output(&entry.output),
    }
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

fn reset_invalid_log_if_needed(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    if load_entries(path).is_ok() {
        return Ok(());
    }

    fs::remove_file(path)
        .with_context(|| format!("Failed to reset invalid session log: {}", path.display()))?;
    let state_path = path.with_extension("state");
    if state_path.exists() {
        let _ = fs::remove_file(state_path);
    }
    Ok(())
}

fn rewrite_sanitized_log_if_needed(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read session log for normalization: {}", path.display()))?;
    if !raw.contains("\\u001b") && !raw.contains("\u{1b}") {
        return Ok(());
    }

    let entries = load_entries(path)?;
    let normalized = entries
        .into_iter()
        .map(|entry| serde_json::to_string(&entry).context("Failed to encode normalized session entry"))
        .collect::<Result<Vec<_>>>()?
        .join("\n");

    let rewritten = if normalized.is_empty() {
        String::new()
    } else {
        format!("{normalized}\n")
    };
    fs::write(path, rewritten)
        .with_context(|| format!("Failed to rewrite sanitized session log: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{extract_output_from_snapshot, render_entry, render_input, SessionEntry};

    #[test]
    fn renders_multiline_prompt_input() {
        let prompt = "repo on main\n❯  ";
        assert_eq!(render_input(prompt, "cargo test"), "repo on main\n❯  cargo test");
    }

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
            extract_output_from_snapshot("sift on 󰊢 main !14 ?2 ⇡1\n❯  ", "cargo test", &lines, 120),
            "ok"
        );
    }

    #[test]
    fn renders_structured_entry() {
        let entry = SessionEntry {
            prompt: "PS C:\\repo> ".to_string(),
            command: "cargo test".to_string(),
            output: "ok".to_string(),
        };

        assert_eq!(render_entry(&entry), "PS C:\\repo> cargo test\nok");
    }

    #[test]
    fn strips_ansi_from_rendered_entry() {
        let entry = SessionEntry {
            prompt: "\x1b[1;32msift\x1b[0m\n\x1b[1;36m❯ \x1b[0m ".to_string(),
            command: "sivtr c 1".to_string(),
            output: "\x1b[92mok\x1b[0m".to_string(),
        };

        assert_eq!(render_entry(&entry), "sift\n❯  sivtr c 1\nok");
    }
}
