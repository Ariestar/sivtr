use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub prompt: String,
    pub command: String,
    pub output: String,
}

impl SessionEntry {
    pub fn new(prompt: impl Into<String>, command: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            prompt: sanitize_prompt(&prompt.into()),
            command: sanitize_command(&command.into()),
            output: sanitize_output(&output.into()),
        }
    }

    pub fn render_input(&self) -> String {
        render_input(&self.prompt, &self.command)
    }

    pub fn render(&self) -> String {
        render_entry(self)
    }
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

    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read session log: {}", path.display()))?;
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
        entries.push(SessionEntry::new(entry.prompt, entry.command, entry.output));
    }

    Ok(entries)
}

pub fn append_entry(path: &Path, entry: &SessionEntry) -> Result<()> {
    reset_invalid_log_if_needed(path)?;
    rewrite_sanitized_log_if_needed(path)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let entry = SessionEntry::new(&entry.prompt, &entry.command, &entry.output);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open session log for append: {}", path.display()))?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&entry).context("Failed to encode session entry")?
    )?;
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
    if prompt.is_empty() {
        return command.to_string();
    }
    if command.is_empty() {
        return prompt.to_string();
    }

    let mut lines: Vec<String> = prompt.lines().map(str::to_string).collect();
    if lines.is_empty() {
        return command.to_string();
    }

    if prompt.ends_with('\n') {
        lines.push(command.to_string());
    } else if let Some(last) = lines.last_mut() {
        last.push_str(command);
    }

    lines.join("\n")
}

pub fn render_entry(entry: &SessionEntry) -> String {
    let input = entry.render_input();

    match (input.is_empty(), entry.output.is_empty()) {
        (false, false) => format!("{input}\n{}", entry.output),
        (false, true) => input,
        (true, false) => entry.output.clone(),
        (true, true) => String::new(),
    }
}

pub fn render_entries(entries: &[SessionEntry]) -> String {
    entries
        .iter()
        .map(SessionEntry::render)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end_matches('\n').to_string()
}

pub(super) fn sanitize_prompt(prompt: &str) -> String {
    normalize_newlines(&strip_ansi_escapes::strip_str(prompt))
}

pub(super) fn sanitize_command(command: &str) -> String {
    normalize_newlines(command)
}

pub(super) fn sanitize_output(output: &str) -> String {
    normalize_newlines(&strip_ansi_escapes::strip_str(output))
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

    let raw = fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read session log for normalization: {}",
            path.display()
        )
    })?;
    if !raw.contains("\\u001b") && !raw.contains("\u{1b}") {
        return Ok(());
    }

    let entries = load_entries(path)?;
    let normalized = entries
        .into_iter()
        .map(|entry| {
            serde_json::to_string(&entry).context("Failed to encode normalized session entry")
        })
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
    use super::{render_entry, render_input, SessionEntry};

    #[test]
    fn renders_multiline_prompt_input() {
        let prompt = "repo on main\n❯  ";
        assert_eq!(render_input(prompt, "cargo test"), "repo on main\n❯  cargo test");
    }

    #[test]
    fn renders_structured_entry() {
        let entry = SessionEntry::new("PS C:\\repo> ", "cargo test", "ok");
        assert_eq!(render_entry(&entry), "PS C:\\repo> cargo test\nok");
    }

    #[test]
    fn strips_ansi_from_entry_at_construction_boundary() {
        let entry = SessionEntry::new(
            "\x1b[1;32msift\x1b[0m\n\x1b[1;36m❯ \x1b[0m ",
            "sivtr c 1",
            "\x1b[92mok\x1b[0m",
        );

        assert_eq!(entry.prompt, "sift\n❯  ");
        assert_eq!(entry.output, "ok");
        assert_eq!(render_entry(&entry), "sift\n❯  sivtr c 1\nok");
    }
}
