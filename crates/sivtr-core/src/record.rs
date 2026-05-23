use crate::ai::{format_blocks, AgentBlock, AgentBlockKind, AgentProvider, AgentSession};
use crate::session::SessionEntry;
use serde::Serialize;
use std::path::Path;

pub const RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRecordKind {
    TerminalCommand,
    ChatTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkChannel {
    Terminal,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOutcome {
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkSource {
    pub channel: WorkChannel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkSessionRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub index: usize,
    pub ref_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkStatus {
    pub outcome: WorkOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkText {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub combined: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkPayload {
    TerminalCommand {
        prompt: String,
        command: String,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_ansi: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_ansi: Option<String>,
    },
    ChatTurn {
        user: String,
        assistant: String,
        messages: Vec<ChatMessage>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkRecord {
    pub schema_version: u32,
    pub id: String,
    pub kind: WorkRecordKind,
    pub source: WorkSource,
    pub session: WorkSessionRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub time: WorkTime,
    pub status: WorkStatus,
    pub title: String,
    pub text: WorkText,
    pub payload: WorkPayload,
}

impl WorkRecord {
    pub fn terminal(entry: &SessionEntry, session_path: &Path, index: usize) -> Option<Self> {
        let command = entry.command.trim().to_string();
        let output = entry.output.trim().to_string();
        if command.is_empty() && output.is_empty() {
            return None;
        }

        let ref_id = format!("terminal/current/{}", index + 1);
        let session_id = session_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("current")
            .to_string();
        let title = if command.is_empty() {
            preview(&output)
        } else {
            preview(&command)
        };
        let outcome = match entry.exit_code {
            Some(0) => WorkOutcome::Success,
            Some(_) => WorkOutcome::Failure,
            None => WorkOutcome::Unknown,
        };
        let combined = join_nonempty([command.as_str(), output.as_str()]);

        Some(Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id: ref_id.clone(),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session_id,
                path: Some(session_path.display().to_string()),
                index,
                ref_id: ref_id.clone(),
            },
            cwd: non_empty(entry.cwd.clone()),
            time: WorkTime {
                occurred_at: entry.ended_at.clone(),
                ended_at: entry.ended_at.clone(),
                duration_ms: entry.duration_ms,
            },
            status: WorkStatus {
                outcome,
                exit_code: entry.exit_code,
            },
            title,
            text: WorkText {
                input: non_empty(Some(command.clone())),
                output: non_empty(Some(output.clone())),
                combined,
            },
            payload: WorkPayload::TerminalCommand {
                prompt: entry.prompt.clone(),
                command,
                output,
                prompt_ansi: entry.prompt_ansi.clone(),
                output_ansi: entry.output_ansi.clone(),
            },
        })
    }

    pub fn chat_turns(provider: AgentProvider, session: &AgentSession) -> Vec<Self> {
        chat_turn_ranges(&session.blocks)
            .into_iter()
            .enumerate()
            .filter_map(|(index, (start, end))| {
                Self::chat_turn(provider, session, index, &session.blocks[start..end])
            })
            .collect()
    }

    fn chat_turn(
        provider: AgentProvider,
        session: &AgentSession,
        index: usize,
        blocks: &[AgentBlock],
    ) -> Option<Self> {
        let messages = blocks
            .iter()
            .filter(|block| matches!(block.kind, AgentBlockKind::User | AgentBlockKind::Assistant))
            .map(|block| ChatMessage {
                role: block_role(block.kind).to_string(),
                content: block.text.trim().to_string(),
                timestamp: block.timestamp.clone(),
            })
            .filter(|message| !message.content.is_empty())
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return None;
        }

        let user = join_nonempty(
            messages
                .iter()
                .filter(|message| message.role == "user")
                .map(|message| message.content.as_str()),
        );
        let assistant = join_nonempty(
            messages
                .iter()
                .filter(|message| message.role == "assistant")
                .map(|message| message.content.as_str()),
        );
        if assistant.trim().is_empty() {
            return None;
        }

        let session_ref = agent_session_ref_id(session.id.as_deref(), &session.path);
        let ref_id = format!("{}/{}/{}", provider.command_name(), session_ref, index + 1);
        let combined = format_blocks(blocks);
        let title = if user.trim().is_empty() {
            preview(&assistant)
        } else {
            preview(&user)
        };
        let occurred_at = first_timestamp(blocks);

        Some(Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id: ref_id.clone(),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some(provider.command_name().to_string()),
            },
            session: WorkSessionRef {
                id: session_ref,
                path: Some(session.path.display().to_string()),
                index,
                ref_id: ref_id.clone(),
            },
            cwd: non_empty(session.cwd.clone()),
            time: WorkTime {
                occurred_at: occurred_at.clone(),
                ended_at: occurred_at,
                duration_ms: None,
            },
            status: WorkStatus {
                outcome: WorkOutcome::Unknown,
                exit_code: None,
            },
            title,
            text: WorkText {
                input: non_empty(Some(user.clone())),
                output: non_empty(Some(assistant.clone())),
                combined,
            },
            payload: WorkPayload::ChatTurn {
                user,
                assistant,
                messages,
            },
        })
    }
}

fn chat_turn_ranges(blocks: &[AgentBlock]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    let mut has_assistant = false;

    for (idx, block) in blocks.iter().enumerate() {
        if block.kind == AgentBlockKind::User && is_real_user_block(block) {
            if let Some(start) = start {
                if has_assistant {
                    ranges.push((start, idx));
                }
            }
            start = Some(idx);
            has_assistant = false;
        } else if start.is_some() && block.kind == AgentBlockKind::Assistant {
            has_assistant = true;
        }
    }

    if let Some(start) = start {
        if has_assistant {
            ranges.push((start, blocks.len()));
        }
    }

    ranges
}

fn is_real_user_block(block: &AgentBlock) -> bool {
    if block.kind != AgentBlockKind::User {
        return false;
    }

    let text = block.text.trim_start();
    !is_agent_startup_user_text(text)
}

fn is_agent_startup_user_text(text: &str) -> bool {
    text.starts_with("# AGENTS.md instructions for")
        || text.starts_with("<environment_context>")
        || text.starts_with("<turn_aborted>")
        || text.starts_with("<local-command-caveat>")
        || text.starts_with("<local-command-stdout>")
        || text.starts_with("<command-message>")
        || text.starts_with("<command-name>")
        || text.starts_with("<ide_opened_file>")
        || text.starts_with("[Request interrupted by user]")
}

fn agent_session_ref_id(id: Option<&str>, path: &Path) -> String {
    id.map(short_id)
        .filter(|id| !id.is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(short_id)
                .filter(|id| !id.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn block_role(kind: AgentBlockKind) -> &'static str {
    match kind {
        AgentBlockKind::User => "user",
        AgentBlockKind::Assistant => "assistant",
        AgentBlockKind::ToolCall => "tool_call",
        AgentBlockKind::ToolOutput => "tool_output",
    }
}

fn first_timestamp(blocks: &[AgentBlock]) -> Option<String> {
    blocks.iter().find_map(|block| block.timestamp.clone())
}

fn preview(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## "))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

fn join_nonempty<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    texts
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| (!value.trim().is_empty()).then_some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn terminal_record_maps_metadata_and_status() {
        let entry = SessionEntry::new("repo> ", "cargo test", "failed").with_metadata(
            Some("D:\\sivtr".to_string()),
            Some("2026-05-23T12:00:00Z".to_string()),
            Some(42),
            Some(101),
        );

        let record = WorkRecord::terminal(&entry, Path::new("session_123.log"), 0).unwrap();

        assert_eq!(record.kind, WorkRecordKind::TerminalCommand);
        assert_eq!(record.source.channel, WorkChannel::Terminal);
        assert_eq!(record.cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(
            record.time.ended_at.as_deref(),
            Some("2026-05-23T12:00:00.000Z")
        );
        assert_eq!(record.time.duration_ms, Some(42));
        assert_eq!(record.status.outcome, WorkOutcome::Failure);
        assert_eq!(record.status.exit_code, Some(101));
        assert_eq!(record.text.input.as_deref(), Some("cargo test"));
        assert_eq!(record.text.output.as_deref(), Some("failed"));
    }

    #[test]
    fn chat_turn_records_ignore_startup_user_blocks() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:00:00Z".to_string()),
                    label: None,
                    text: "<environment_context>".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    text: "implement this".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: None,
                    text: "done".to_string(),
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Pi, &session);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, WorkRecordKind::ChatTurn);
        assert_eq!(records[0].source.channel, WorkChannel::Chat);
        assert_eq!(records[0].source.provider.as_deref(), Some("pi"));
        assert_eq!(records[0].text.input.as_deref(), Some("implement this"));
        assert_eq!(records[0].text.output.as_deref(), Some("done"));
        assert_eq!(
            records[0].time.occurred_at.as_deref(),
            Some("2026-05-23T12:01:00Z")
        );
    }
}
