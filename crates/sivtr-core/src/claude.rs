use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::ai::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionInfo,
    AgentSessionMeta, AgentSessionProvider,
};

const PROVIDER_NAME: &str = "Claude";

#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeProvider;

impl AgentSessionProvider for ClaudeProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Claude
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_recent_jsonl_sessions(&claude_home().join("projects"), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn claude_home() -> PathBuf {
    if let Ok(path) = std::env::var("CLAUDE_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 50, update_meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value) {
    if meta.id.is_none() {
        meta.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if meta.cwd.is_none() {
        meta.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    update_session_meta(session, value);

    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return;
    }

    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);

    match value.get("type").and_then(Value::as_str) {
        Some("user") => apply_message(session, value, AgentBlockKind::User, timestamp),
        Some("assistant") => apply_message(session, value, AgentBlockKind::Assistant, timestamp),
        Some(
            "permission-mode"
            | "ai-title"
            | "file-history-snapshot"
            | "attachment"
            | "last-prompt"
            | "system"
            | "queue-operation",
        ) => {}
        Some(event_type) => {
            panic!("Unexpected Claude event type: {event_type}");
        }
        None => {
            panic!("Claude event must include type");
        }
    }
}

fn update_session_meta(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
}

fn apply_message(
    session: &mut AgentSession,
    value: &Value,
    kind: AgentBlockKind,
    timestamp: Option<String>,
) {
    let content = value
        .get("message")
        .and_then(|message| message.get("content"))
        .expect("Claude user/assistant event must include message.content");
    push_content_blocks(session, kind, timestamp, content);
}

fn push_content_blocks(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    match content {
        Value::Array(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => push_text_content_blocks(
                        session,
                        kind,
                        timestamp.clone(),
                        extract_content_text(item),
                    ),
                    Some("thinking") => {}
                    Some("tool_use") => push_tool_use(session, timestamp.clone(), item),
                    Some("tool_result") => {
                        let content = item
                            .get("content")
                            .expect("Claude structured tool_result must include content");
                        push_block(
                            session,
                            AgentBlockKind::ToolOutput,
                            timestamp.clone(),
                            None,
                            extract_content_text(content),
                        );
                    }
                    Some("image") => {}
                    None => {
                        panic!("Claude content item must include type");
                    }
                    Some(item_type) => {
                        panic!("Unexpected Claude content item type: {item_type}");
                    }
                }
            }
        }
        Value::String(text) => push_text_content_blocks(session, kind, timestamp, text.clone()),
        other => {
            panic!("Unexpected Claude message.content shape: {other:?}");
        }
    }
}

fn push_tool_use(session: &mut AgentSession, timestamp: Option<String>, item: &Value) {
    let input = item
        .get("input")
        .expect("Claude structured tool_use must include input");
    push_block(
        session,
        AgentBlockKind::ToolCall,
        timestamp,
        item.get("name").and_then(Value::as_str).map(str::to_string),
        pretty_json_value(input),
    );
}

fn push_text_content_blocks(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    text: String,
) {
    if kind == AgentBlockKind::Assistant {
        push_assistant_text_blocks(session, timestamp, &text);
    } else {
        push_block(session, kind, timestamp, None, text);
    }
}

fn push_assistant_text_blocks(session: &mut AgentSession, timestamp: Option<String>, text: &str) {
    let mut rest = text;

    rest = skip_completed_tool_close_tags(rest);
    while let Some((start, tool_kind)) = find_next_embedded_tool(rest) {
        push_block(
            session,
            AgentBlockKind::Assistant,
            timestamp.clone(),
            None,
            &rest[..start],
        );

        let parsed = parse_embedded_tool_span(&rest[start..], tool_kind);
        push_block(
            session,
            tool_kind.block_kind(),
            timestamp.clone(),
            parsed.name,
            tool_child_body(parsed.inner, tool_kind),
        );

        rest = &rest[start + parsed.consumed..];
        rest = skip_completed_tool_close_tags(rest);
    }

    rest = skip_completed_tool_close_tags(rest);
    push_block(session, AgentBlockKind::Assistant, timestamp, None, rest);
}

#[derive(Clone, Copy)]
enum EmbeddedToolKind {
    ToolUse,
    ToolResult,
}

impl EmbeddedToolKind {
    fn open_tag(self) -> &'static str {
        match self {
            Self::ToolUse => "<tool_use",
            Self::ToolResult => "<tool_result",
        }
    }

    fn close_tag(self) -> &'static str {
        match self {
            Self::ToolUse => "</tool_use>",
            Self::ToolResult => "</tool_result>",
        }
    }

    fn child_tag(self) -> &'static str {
        match self {
            Self::ToolUse => "tool_input",
            Self::ToolResult => "tool_output",
        }
    }

    fn block_kind(self) -> AgentBlockKind {
        match self {
            Self::ToolUse => AgentBlockKind::ToolCall,
            Self::ToolResult => AgentBlockKind::ToolOutput,
        }
    }
}

struct EmbeddedTool<'a> {
    name: Option<String>,
    inner: &'a str,
    consumed: usize,
}

fn find_next_embedded_tool(text: &str) -> Option<(usize, EmbeddedToolKind)> {
    [EmbeddedToolKind::ToolUse, EmbeddedToolKind::ToolResult]
        .into_iter()
        .filter_map(|kind| text.find(kind.open_tag()).map(|index| (index, kind)))
        .min_by_key(|(index, _)| *index)
}

fn parse_embedded_tool_span<'a>(text: &'a str, tool_kind: EmbeddedToolKind) -> EmbeddedTool<'a> {
    let open_end = text
        .find('>')
        .expect("Claude embedded tool tag must have an opening delimiter");
    let opening = &text[..=open_end];
    let body_start = open_end + 1;
    let close = tool_kind.close_tag();
    let close_start = text[body_start..]
        .find(close)
        .map(|offset| body_start + offset);
    let next_tool_start =
        find_next_embedded_tool(&text[body_start..]).map(|(offset, _)| body_start + offset);
    let body_end = match (close_start, next_tool_start) {
        (Some(close_start), Some(next_tool_start)) => close_start.min(next_tool_start),
        (Some(close_start), None) => close_start,
        (None, Some(next_tool_start)) => next_tool_start,
        (None, None) => text.len(),
    };
    let consumed = if close_start == Some(body_end) {
        body_end + close.len()
    } else {
        body_end
    };

    EmbeddedTool {
        name: tool_name_attr(opening),
        inner: &text[body_start..body_end],
        consumed,
    }
}

fn skip_completed_tool_close_tags(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim_start();
        let mut stripped = false;

        for close in [
            "</tool_input>",
            "</tool_output>",
            "</tool_use>",
            "</tool_result>",
        ] {
            if let Some(rest) = trimmed.strip_prefix(close) {
                text = rest;
                stripped = true;
                break;
            }
        }

        if !stripped {
            return text;
        }
    }
}

fn tool_name_attr(opening: &str) -> Option<String> {
    let rest = opening.split_once("name=\"")?.1;
    let name = rest.split_once('"')?.0.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn tool_child_body(inner: &str, tool_kind: EmbeddedToolKind) -> String {
    let child = tool_kind.child_tag();
    let open_start = match inner.find(&format!("<{child}")) {
        Some(index) => index,
        None => panic!("Claude embedded tool missing <{child}>"),
    };
    let open_end = match inner[open_start..].find('>') {
        Some(offset) => open_start + offset,
        None => panic!("Claude embedded tool <{child}> must have a delimiter"),
    };
    let close = format!("</{child}>");
    let body_start = open_end + 1;
    let close_start = match inner[body_start..].find(&close) {
        Some(offset) => body_start + offset,
        None => inner.len(),
    };

    inner[body_start..close_start].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::ClaudeProvider;
    use crate::ai::{
        format_blocks, select_blocks, AgentBlockKind, AgentSelection, AgentSessionProvider,
    };

    #[test]
    fn parses_claude_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:00Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"I will check."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi"}}]}}
{"type":"user","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"hi"}]}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
    }

    #[test]
    fn ignores_claude_ai_title_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","message":{"role":"user","content":"hello"}}
{"type":"ai-title","sessionId":"abc","cwd":"C:\\repo","title":"Short title"}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(
            format_blocks(&session.blocks),
            "## User\nhello\n\n## Assistant\ndone"
        );
    }

    #[test]
    fn parses_claude_embedded_tool_markup_from_text_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","message":{"role":"user","content":"review"}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk ls\"}</tool_input>\n</tool_use><tool_result name=\"Bash\">\n  <tool_output>ok</tool_output>\n</tool_result>\nfinal answer"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("Bash"));
        assert_eq!(session.blocks[1].text, "{\"command\":\"rtk ls\"}");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[2].text, "ok");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "final answer");
    }

    #[test]
    fn parses_truncated_embedded_tool_output_at_next_tool_tag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"before<tool_result name=\"Bash\">\n  <tool_output>partial output\nNeed next.<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk test\"}</tool_input>\n</tool_use>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "before");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert!(session.blocks[2].text.contains("partial output"));
        assert!(!session.blocks[2].text.contains("<tool_result"));
        assert!(!session.blocks[2].text.contains("<tool_output"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[4].text, "after");
    }

    #[test]
    fn splits_embedded_tool_when_next_tool_tag_precedes_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"<tool_result name=\"Bash\">\n  <tool_output>partial output<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk test\"}</tool_input>\n</tool_use></tool_result>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[1].text, "partial output");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].text, "{\"command\":\"rtk test\"}");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "after");
    }

    #[test]
    fn parses_embedded_tool_input_without_child_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk test\"}\n</tool_use>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].text, "{\"command\":\"rtk test\"}");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "after");
    }

    #[test]
    fn skips_claude_meta_user_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"hidden command caveat"}}
{"type":"user","message":{"role":"user","content":"real task"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "real task");
        assert_eq!(session.blocks[1].text, "done");
    }

    #[test]
    fn selects_last_claude_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"first"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"old"}]}}
{"type":"user","message":{"role":"user","content":"second"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"new"}]}}
"#,
        )
        .unwrap();
        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastTurn);

        assert_eq!(blocks.len(), 2);
        assert_eq!(
            format_blocks(&blocks),
            "## User\nsecond\n\n## Assistant\nnew"
        );
    }
}
