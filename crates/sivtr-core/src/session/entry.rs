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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_ansi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ansi: Option<String>,
}

impl SessionEntry {
    pub fn new(
        prompt: impl Into<String>,
        command: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self::from_raw(prompt.into(), command.into(), output.into())
    }

    pub fn render_input(&self) -> String {
        render_input(&self.prompt, &self.command)
    }

    pub fn render_input_ansi(&self) -> String {
        render_input(
            self.prompt_ansi.as_deref().unwrap_or(&self.prompt),
            &self.command,
        )
    }

    pub fn render(&self) -> String {
        render_entry(self)
    }

    pub fn render_ansi(&self) -> String {
        render_entry_ansi(self)
    }

    pub fn has_ansi(&self) -> bool {
        self.prompt_ansi.is_some() || self.output_ansi.is_some()
    }

    fn from_raw(prompt: String, command: String, output: String) -> Self {
        let prompt = normalize_newlines(&prompt);
        let command = normalize_newlines(&command);
        let output = normalize_newlines(&output);
        let prompt_plain = sanitize_prompt(&prompt);
        let output_plain = sanitize_output(&output);

        Self {
            prompt: prompt_plain.clone(),
            command: sanitize_command(&command),
            output: output_plain.clone(),
            prompt_ansi: preserve_ansi(prompt, &prompt_plain),
            output_ansi: preserve_ansi(output, &output_plain),
        }
    }

    fn normalized(self) -> Self {
        let raw_prompt = normalize_newlines(&self.prompt);
        let raw_output = normalize_newlines(&self.output);
        let prompt_plain = sanitize_prompt(&raw_prompt);
        let output_plain = sanitize_output(&raw_output);

        Self {
            prompt: prompt_plain.clone(),
            command: sanitize_command(&self.command),
            output: output_plain.clone(),
            prompt_ansi: self
                .prompt_ansi
                .and_then(|prompt| preserve_ansi(prompt, &prompt_plain))
                .or_else(|| preserve_ansi(raw_prompt, &prompt_plain)),
            output_ansi: self
                .output_ansi
                .and_then(|output| preserve_ansi(output, &output_plain))
                .or_else(|| preserve_ansi(raw_output, &output_plain)),
        }
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
            format!(
                "Failed to read session log line {}: {}",
                idx + 1,
                path.display()
            )
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
        entries.push(entry.normalized());
    }

    Ok(entries)
}

pub fn append_entry(path: &Path, entry: &SessionEntry) -> Result<()> {
    reset_invalid_log_if_needed(path)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let entry = SessionEntry::from_raw(
        entry
            .prompt_ansi
            .clone()
            .unwrap_or_else(|| entry.prompt.clone()),
        entry.command.clone(),
        entry
            .output_ansi
            .clone()
            .unwrap_or_else(|| entry.output.clone()),
    );
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
    render_entry_parts(&entry.render_input(), &entry.output)
}

pub fn render_entry_ansi(entry: &SessionEntry) -> String {
    render_entry_parts(
        &entry.render_input_ansi(),
        entry.output_ansi.as_deref().unwrap_or(&entry.output),
    )
}

pub fn render_entries(entries: &[SessionEntry]) -> String {
    entries
        .iter()
        .map(SessionEntry::render)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_entries_ansi(entries: &[SessionEntry]) -> String {
    entries
        .iter()
        .map(SessionEntry::render_ansi)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_entry_parts(input: &str, output: &str) -> String {
    match (input.is_empty(), output.is_empty()) {
        (false, false) => format!("{input}\n{output}"),
        (false, true) => input.to_string(),
        (true, false) => output.to_string(),
        (true, true) => String::new(),
    }
}

pub(super) fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string()
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

fn preserve_ansi(raw: String, plain: &str) -> Option<String> {
    let normalized = normalize_newlines(&raw);
    if normalized.is_empty() || normalized == plain {
        None
    } else {
        Some(normalized)
    }
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

#[cfg(test)]
mod tests {
    use super::{render_entries_ansi, render_entry, render_entry_ansi, render_input, SessionEntry};

    #[test]
    fn renders_multiline_prompt_input() {
        let prompt = "repo on main\n❯  ";
        assert_eq!(
            render_input(prompt, "cargo test"),
            "repo on main\n❯  cargo test"
        );
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
        assert_eq!(
            entry.prompt_ansi.as_deref(),
            Some("\x1b[1;32msift\x1b[0m\n\x1b[1;36m❯ \x1b[0m ")
        );
        assert_eq!(entry.output_ansi.as_deref(), Some("\x1b[92mok\x1b[0m"));
        assert_eq!(render_entry(&entry), "sift\n❯  sivtr c 1\nok");
        assert_eq!(
            render_entry_ansi(&entry),
            "\x1b[1;32msift\x1b[0m\n\x1b[1;36m❯ \x1b[0m sivtr c 1\n\x1b[92mok\x1b[0m"
        );
    }

    #[test]
    fn renders_ansi_entries_with_fallback() {
        let entries = vec![
            SessionEntry::new(
                "\x1b[36mPS C:\\repo>\x1b[0m ",
                "cargo test",
                "\x1b[31mfailed\x1b[0m",
            ),
            SessionEntry::new("PS C:\\repo> ", "cargo check", "ok"),
        ];

        assert_eq!(
            render_entries_ansi(&entries),
            "\x1b[36mPS C:\\repo>\x1b[0m cargo test\n\x1b[31mfailed\x1b[0m\nPS C:\\repo> cargo check\nok"
        );
    }
}
