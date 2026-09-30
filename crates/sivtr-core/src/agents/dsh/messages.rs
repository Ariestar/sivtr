use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashMap;

use crate::agents::{
    extract_content_text, pretty_json_string, push_block, push_tool_block, AgentBlockKind,
    AgentSession,
};

/// Admit the fields used by the transcript reader, without implementing the
/// harness's model-history, lifecycle, or plugin state reconstruction.
pub(super) fn validate_v4_event(value: &Value) -> Result<()> {
    let event_type = value.get("type").and_then(Value::as_str);
    if !event_type.is_some_and(is_known_v4_event) {
        if value.get("ignorable").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        bail!("unsupported Dsh v4 event type {event_type:?}");
    }
    let role = match event_type {
        Some("user/message") => "user",
        Some("assistant/message") => "assistant",
        Some("tool/result") => "tool",
        _ => return Ok(()),
    };
    let message = if role == "user" {
        value.get("data")
    } else {
        value.pointer("/data/message")
    };
    let Some(message) = message.filter(|message| message.is_object()) else {
        bail!("Dsh v4 {role} message is missing its payload");
    };
    let kind = message.pointer("/source/kind").and_then(Value::as_str);
    if !kind.is_some_and(|kind| !kind.is_empty() && kind != "plugin")
        || (role == "assistant" && kind != Some("model"))
        || (role == "tool" && kind != Some("tool"))
    {
        bail!("Dsh v4 {role} message requires native source attribution");
    }
    if message.get("role").and_then(Value::as_str) != Some(role)
        || !message
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
    {
        bail!("Dsh v4 {role} message requires its native role and id");
    }
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        bail!("Dsh v4 {role} message requires array content");
    };
    if content
        .iter()
        .any(|part| part.get("type").and_then(Value::as_str) == Some("tool-result"))
    {
        bail!("Dsh v4 messages must not contain retired tool-result wrappers");
    }
    let surface = value.get("surfaceOp");
    if surface.and_then(Value::as_str) != Some("append")
        && !surface.is_some_and(|op| {
            op.get("op").and_then(Value::as_str) == Some("replace")
                && op.get("startSeq").and_then(Value::as_u64).is_some()
                && op.get("endSeq").and_then(Value::as_u64).is_some()
        })
    {
        bail!("Dsh v4 {role} message requires an append or replace surface operation");
    }
    if role == "tool" {
        let call_id = message.get("toolCallId").and_then(Value::as_str);
        if !call_id.is_some_and(|id| !id.is_empty())
            || call_id != message.pointer("/source/callId").and_then(Value::as_str)
        {
            bail!("Dsh v4 tool result requires matching toolCallId and source.callId");
        }
        if message
            .get("isError")
            .is_some_and(|flag| !flag.is_boolean())
            || (value.pointer("/data/error").is_some()
                && message.get("isError").and_then(Value::as_bool) != Some(true))
        {
            bail!("Dsh v4 tool result has inconsistent error metadata");
        }
    }
    Ok(())
}

/// V4 vocabulary from dsh's core/session/src/known-event-types.ts. Known
/// context/trace records do not enter the human transcript. Unknown required
/// events must fail rather than silently dropping a future reconstruction rule.
fn is_known_v4_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "agent-preset/selected"
            | "agent/inbox/spliced"
            | "approval/asked"
            | "approval/decided"
            | "approval/policy"
            | "assistant/attempt"
            | "assistant/message"
            | "command/done"
            | "command/run"
            | "compaction/end"
            | "compaction/prune"
            | "compaction/start"
            | "compaction/summary"
            | "deliverables/presented"
            | "developer/message"
            | "feedback/message-delete"
            | "feedback/message-put"
            | "feedback/record"
            | "goal/change"
            | "hook/invoked"
            | "hook/result"
            | "image/offload"
            | "llm/retry"
            | "llm/retry-started"
            | "model/selection"
            | "permission/preset"
            | "plan/mode"
            | "request/context"
            | "request/header"
            | "sandbox/mode"
            | "schedule/change"
            | "session-log-deepseek/delivery-accepted"
            | "session/end-seed"
            | "session/title"
            | "session/title-llm-request"
            | "step/end"
            | "step/start"
            | "subagent/catalog"
            | "subagent/descriptor"
            | "subagent/model-selection-policy"
            | "system/message"
            | "team/member"
            | "team/message/delivered"
            | "team/message/queued"
            | "team/task"
            | "todo/write"
            | "tool-workflow/agent-end"
            | "tool-workflow/agent-start"
            | "tool-workflow/run-end"
            | "tool-workflow/run-start"
            | "tool/call"
            | "tool/ptc-dispatch"
            | "tool/ptc-dispatch-start"
            | "tool/result"
            | "turn/end"
            | "turn/start"
            | "user/message"
            | "web/deepseek-search-llm-request"
            | "workspace/changes"
    )
}

pub(super) fn apply_event(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    first_user: &mut Option<String>,
    value: &Value,
    version: u64,
) {
    if !is_append_message(value, version) {
        return;
    }
    let timestamp = value
        .get("time")
        .and_then(Value::as_i64)
        .map(|ms| ms.to_string());
    match value.get("type").and_then(Value::as_str) {
        Some("session/title") => {
            // dsh titles are latest-wins snapshots; keep the last one seen.
            if let Some(title) = event_title(value) {
                session.title = Some(title);
            }
        }
        Some("user/message") => {
            if !is_user_message(value, version) {
                return;
            }
            let Some(text) = user_message_text(value) else {
                return;
            };
            if first_user.is_none() {
                *first_user = Some(text.clone());
            }
            push_block(session, AgentBlockKind::User, timestamp, None, text);
        }
        Some("assistant/message") => {
            let Some(message) = value.pointer("/data/message") else {
                return;
            };
            apply_assistant_message(session, tool_names, timestamp, message);
        }
        Some("tool/call") => {
            if let (Some(call_id), Some(name)) = (
                value.pointer("/data/callId").and_then(Value::as_str),
                value.pointer("/data/name").and_then(Value::as_str),
            ) {
                tool_names.insert(call_id.to_string(), name.to_string());
            }
        }
        Some("tool/result") => {
            let Some(message) = value.pointer("/data/message") else {
                return;
            };
            apply_tool_result_message(session, tool_names, timestamp, message, version);
        }
        _ => {}
    }
}

pub(super) fn event_title(value: &Value) -> Option<String> {
    value
        .pointer("/data/title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

/// dsh projects several synthetic user-role messages onto the surface:
/// runtime-context snapshots (`plugin`), workspace-instruction and
/// skill-catalog injections (`agent-instructions`, `skill-catalog`), cron
/// notices, and goal continuations. Only direct human prompts
/// (`source.kind == "user"`, or a missing source) belong in the dialogue —
/// the rest is runtime context that would pollute search.
pub(super) fn is_user_message(value: &Value, version: u64) -> bool {
    match value.pointer("/data/source/kind").and_then(Value::as_str) {
        Some("user") => true,
        None => version == 0,
        _ => false,
    }
}

pub(super) fn is_append_message(value: &Value, version: u64) -> bool {
    version == 0
        || !matches!(
            value.get("type").and_then(Value::as_str),
            Some("user/message" | "assistant/message" | "tool/result")
        )
        || value.get("surfaceOp").and_then(Value::as_str) == Some("append")
}

pub(super) fn user_message_text(value: &Value) -> Option<String> {
    let text = value
        .pointer("/data/content")
        .map(extract_content_text)
        .unwrap_or_default();
    (!text.trim().is_empty()).then_some(text)
}

fn apply_assistant_message(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    timestamp: Option<String>,
    message: &Value,
) {
    let Some(content) = message.get("content") else {
        return;
    };
    let Value::Array(parts) = content else {
        return;
    };
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => push_block(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                None,
                part.get("text").and_then(Value::as_str).unwrap_or_default(),
            ),
            Some("reasoning") => push_block(
                session,
                AgentBlockKind::Thinking,
                timestamp.clone(),
                None,
                part.get("text").and_then(Value::as_str).unwrap_or_default(),
            ),
            Some("tool-call") => {
                let call_id = part.get("id").and_then(Value::as_str).map(str::to_string);
                let name = part.get("name").and_then(Value::as_str).map(str::to_string);
                let arguments = part
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(pretty_json_string)
                    .unwrap_or_default();
                if let (Some(call_id), Some(name)) = (call_id.as_deref(), name.as_deref()) {
                    tool_names.insert(call_id.to_string(), name.to_string());
                }
                push_tool_block(
                    session,
                    AgentBlockKind::ToolCall,
                    timestamp.clone(),
                    call_id,
                    name,
                    arguments,
                    None,
                );
            }
            Some("tool-result") => {
                apply_tool_result_part(session, tool_names, timestamp.clone(), part)
            }
            _ => {}
        }
    }
}

fn apply_tool_result_message(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    timestamp: Option<String>,
    message: &Value,
    version: u64,
) {
    if version == 4 {
        // V4 moves toolCallId/content from the retired wrapper onto the message;
        // the existing block conversion already consumes exactly these fields.
        apply_tool_result_part(session, tool_names, timestamp, message);
        return;
    }
    let Some(content) = message.get("content") else {
        return;
    };
    let Value::Array(parts) = content else {
        return;
    };
    for part in parts {
        if part.get("type").and_then(Value::as_str) == Some("tool-result") {
            apply_tool_result_part(session, tool_names, timestamp.clone(), part);
        }
    }
}

fn apply_tool_result_part(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    timestamp: Option<String>,
    part: &Value,
) {
    let call_id = part
        .get("toolCallId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let label = call_id
        .as_deref()
        .and_then(|id| tool_names.get(id).cloned());
    let text = part
        .get("content")
        .map(extract_content_text)
        .unwrap_or_default();
    push_tool_block(
        session,
        AgentBlockKind::ToolOutput,
        timestamp,
        call_id,
        label,
        text,
        None,
    );
}
