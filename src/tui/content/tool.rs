//! Shape-driven tool display for the content pane.
//!
//! Tool calls get per-tool names and input expressions instead of the generic
//! `<:tool:Name call:>` marker: `<:read: src/main.rs:12-30:>`, expanded to a
//! `$` input line with a code/diff preview, and `>` output lines. The
//! formatter keys off the normalized `tool` name and the `input` / `output`
//! JSON, so every provider (claude, grok, codex, …) flows through one code
//! path — display only; evidence export keeps its original markers.

use serde_json::Value;
use sivtr_core::record::{WorkPart, WorkPartData};

/// Long input expressions are truncated to fit a tag line.
const MAX_EXPR: usize = 40;

/// Canonical short name for known tools (`Read` → `read`, `apply_patch` →
/// `patch`); `None` for unknown tools that keep the generic tool marker.
/// Covers provider tool names: claude (`Read`), opencode (`read`), codex
/// (`apply_patch`), grok build (`read_file`, `run_terminal_command`,
/// `search_replace`).
fn known_name(tool: &str) -> Option<&'static str> {
    match tool.to_ascii_lowercase().as_str() {
        "read" | "read_file" => Some("read"),
        "grep" | "search_files" => Some("grep"),
        "edit" | "search_replace" => Some("edit"),
        "write" => Some("write"),
        "bash" | "shell" | "run_terminal_command" => Some("bash"),
        "apply_patch" | "patch" => Some("patch"),
        "webfetch" | "web_fetch" => Some("webfetch"),
        "websearch" | "web_search" => Some("websearch"),
        "notebookedit" | "notebook_edit" => Some("notebook-edit"),
        _ => None,
    }
}

/// Grok's MCP dispatcher: `use_tool` calls a `server__tool` by name.
fn is_use_tool(tool: &str) -> bool {
    tool.eq_ignore_ascii_case("use_tool")
}

/// Display name for any tool: MCP tools as `server: tool`, known tools
/// canonically, unknown tools as their lowercased name.
pub(crate) fn tool_display_name(tool: &str) -> String {
    if let Some(rest) = tool.strip_prefix("mcp__") {
        if let Some((server, name)) = rest.split_once("__") {
            return format!("{server}: {name}");
        }
    }
    known_name(tool)
        .map(str::to_string)
        .unwrap_or_else(|| tool.to_ascii_lowercase())
}

/// Whether the tool gets the new per-tool rendering (known names + MCP).
fn is_known_tool(tool: &str) -> bool {
    known_name(tool).is_some() || tool.starts_with("mcp__") || is_use_tool(tool)
}

/// Display name of a call: `use_tool` shows its target tool, others the
/// tool's own name.
fn tool_call_name(tool: &str, input: &Value) -> String {
    if is_use_tool(tool) {
        use_tool_name(input).unwrap_or_else(|| tool_display_name(tool))
    } else {
        tool_display_name(tool)
    }
}

/// Folded tag for a tool *call*: MCP tools always (`<:sivtr: sivtr_search:>`),
/// `use_tool` as its target tool, known tools when the input shape is
/// understood (`<:bash: ls:>`); `None` keeps the generic `<:tool:Name call:>`
/// marker.
pub(crate) fn tool_tag(tool: &str, value: &Value) -> Option<String> {
    if tool.starts_with("mcp__") || is_use_tool(tool) {
        return Some(format!("<:{}:>", tool_call_name(tool, value)));
    }
    let name = known_name(tool)?;
    if name == "patch" {
        // apply_patch always carries a raw patch string; no short expr.
        return Some(format!("<:{name}:>"));
    }
    let expr = tool_input_expr(tool, value)?;
    Some(format!("<:{name}: {expr}:>"))
}

/// New-style folded tag for a tool part, when the tool is known: calls get
/// the expression tag, results the bare name tag.
pub(crate) fn tool_tag_for_part(part: &WorkPart) -> Option<String> {
    match &part.data {
        WorkPartData::ToolCall { tool, input, .. } => {
            tool_tag(tool.as_deref().unwrap_or_default(), input)
        }
        WorkPartData::ToolResult { tool, .. } => {
            let tool = tool.as_deref().unwrap_or_default();
            if is_known_tool(tool) {
                Some(format!("<:{}:>", tool_display_name(tool)))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Whether the new `$` format applies to a tool call: MCP tools and
/// `use_tool` always, known tools when the input shape is understood (an
/// expression or a preview). Unrecognized shapes keep the evidence format so
/// no payload is lost.
fn tool_renderable_call(tool: &str, input: &Value) -> bool {
    tool.starts_with("mcp__")
        || is_use_tool(tool)
        || tool_input_expr(tool, input).is_some()
        || diff_preview(tool, input).is_some()
}

/// Display body of one part: the `$`/`>` tool format for understood tool
/// shapes, the evidence format otherwise.
pub(crate) fn part_body_text(part: &WorkPart) -> String {
    match &part.data {
        WorkPartData::ToolCall { tool, input, .. } => {
            let tool = tool.as_deref().unwrap_or_default();
            if tool_renderable_call(tool, input) {
                tool_call_text(tool, input)
            } else {
                sivtr_core::record::format_work_part(part)
            }
        }
        WorkPartData::ToolResult { tool, output, .. } => {
            let tool = tool.as_deref().unwrap_or_default();
            if is_known_tool(tool) {
                tool_result_text(tool, output)
            } else {
                sivtr_core::record::format_work_part(part)
            }
        }
        _ => sivtr_core::record::format_work_part(part),
    }
}

/// Display name for `use_tool` (grok's MCP dispatcher): the `tool_name`
/// argument, `server__tool` rendered as `server: tool`.
fn use_tool_name(input: &Value) -> Option<String> {
    let tool_name = input.as_object()?.get("tool_name")?.as_str()?;
    Some(match tool_name.split_once("__") {
        Some((server, name)) => format!("{server}: {name}"),
        None => tool_name.to_string(),
    })
}

/// Input expression from a tool call's input JSON: `src/main.rs:12-30`,
/// `export function foo`, `cd src && make`… `None` when the shape is unknown.
fn tool_input_expr(tool: &str, input: &Value) -> Option<String> {
    let name = known_name(tool)?;
    if name == "patch" {
        return None; // apply_patch carries a raw patch string, no short expr.
    }
    let obj = input.as_object()?;
    let expr = match name {
        "read" => {
            let path = path_field(obj)?;
            match line_range(obj) {
                Some(range) => format!("{path}:{range}"),
                None => path.to_string(),
            }
        }
        "grep" => obj.get("pattern")?.as_str()?.trim().to_string(),
        "edit" | "write" => path_field(obj)?.to_string(),
        "bash" => obj.get("command")?.as_str()?.trim().to_string(),
        "webfetch" | "websearch" => obj
            .get("url")
            .or_else(|| obj.get("query"))?
            .as_str()?
            .trim()
            .to_string(),
        _ => return None,
    };
    if expr.is_empty() {
        return None;
    }
    Some(truncate(expr))
}

/// File path field: `file_path` (claude/grok), `filePath` (opencode),
/// `target_file` (grok build), or `path` (codex).
fn path_field(obj: &serde_json::Map<String, Value>) -> Option<&str> {
    obj.get("file_path")
        .or_else(|| obj.get("filePath"))
        .or_else(|| obj.get("target_file"))
        .or_else(|| obj.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// Line range: grok-style `line_start`/`line_end` (1-based inclusive), else
/// claude-style `offset` (0-based) / `limit` lines.
fn line_range(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let start = obj.get("line_start").and_then(Value::as_i64);
    let end = obj.get("line_end").and_then(Value::as_i64);
    match (start, end) {
        (Some(start), Some(end)) => Some(format!("{start}-{end}")),
        _ => {
            let offset = obj.get("offset").and_then(Value::as_i64);
            let limit = obj.get("limit").and_then(Value::as_i64);
            match (offset, limit) {
                (Some(offset), Some(limit)) => Some(format!("{}-{}", offset + 1, offset + limit)),
                (Some(offset), None) => Some(format!("{}", offset + 1)),
                _ => None,
            }
        }
    }
}

/// Expanded body of a tool call: the `$` input line, plus a diff preview
/// for write/edit (which carry content in the input).
pub(crate) fn tool_call_text(tool: &str, input: &Value) -> String {
    let name = tool_call_name(tool, input);
    let expr = tool_input_expr(tool, input);
    let line = if known_name(tool) == Some("bash") {
        // `$` is the shell prompt: the command is the whole instruction.
        format!("$ {}", expr.unwrap_or(name))
    } else {
        match expr {
            Some(expr) => format!("$ {name} {expr}"),
            None => format!("$ {name}"),
        }
    };
    match diff_preview(tool, input) {
        Some(preview) => format!("{line}\n{preview}"),
        None => line,
    }
}

/// Diff preview from the tool input: `+` lines for write content, `-`/`+`
/// for edit old/new strings, inside a ```diff fence the pane colors.
fn diff_preview(tool: &str, input: &Value) -> Option<String> {
    let name = known_name(tool)?;
    if name == "patch" {
        // apply_patch (codex) carries the whole unified diff as a string.
        let Value::String(patch) = input else {
            return None;
        };
        let patch = patch.trim_end();
        if patch.is_empty() {
            return None;
        }
        return Some(format!("```diff\n{patch}\n```"));
    }
    let obj = input.as_object()?;
    let mut diff = Vec::new();
    match name {
        "write" => {
            let content = obj.get("content")?.as_str()?;
            diff.extend(content.lines().map(|line| format!("+ {line}")));
        }
        "edit" => {
            if let Some(old_string) = obj
                .get("old_string")
                .or_else(|| obj.get("oldString"))
                .and_then(Value::as_str)
            {
                diff.extend(old_string.lines().map(|line| format!("- {line}")));
            }
            if let Some(new_string) = obj
                .get("new_string")
                .or_else(|| obj.get("newString"))
                .and_then(Value::as_str)
            {
                diff.extend(new_string.lines().map(|line| format!("+ {line}")));
            }
        }
        _ => return None,
    }
    if diff.is_empty() {
        return None;
    }
    Some(format!("```diff\n{}\n```", diff.join("\n")))
}

/// Expanded body of a tool result: `>` output lines, or a code block for
/// read (the file content preview shown right under the `$ read` line).
pub(crate) fn tool_result_text(tool: &str, output: &Value) -> String {
    let text = match output {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(output).unwrap_or_default(),
    };
    if known_name(tool) == Some("read") {
        format!("```\n{}\n```", text.trim_end())
    } else {
        text.lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Truncate a long expression to fit a tag line, appending `…`.
fn truncate(text: String) -> String {
    if text.chars().count() <= MAX_EXPR {
        return text;
    }
    let mut out: String = text.chars().take(MAX_EXPR).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str, input: serde_json::Value) -> WorkPart {
        WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some(tool.to_string()),
                input,
            },
        }
    }

    fn result(tool: &str, output: serde_json::Value) -> WorkPart {
        WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: None,
                tool: Some(tool.to_string()),
                output,
            },
        }
    }

    #[test]
    fn mcp_tools_render_as_server_colon_tool() {
        assert_eq!(
            tool_display_name("mcp__sivtr__sivtr_search"),
            "sivtr: sivtr_search"
        );
        assert_eq!(
            tool_tag("mcp__codegraph__codegraph_context", &Value::Null).unwrap(),
            "<:codegraph: codegraph_context:>"
        );
    }

    #[test]
    fn known_tools_get_canonical_names_and_exprs() {
        let read = call(
            "Read",
            serde_json::json!({"file_path": "src/main.rs", "offset": 11, "limit": 19}),
        );
        assert_eq!(
            tool_tag_for_part(&read).unwrap(),
            "<:read: src/main.rs:12-30:>"
        );
        assert_eq!(part_body_text(&read), "$ read src/main.rs:12-30");

        let read_grok = call(
            "read",
            serde_json::json!({"file_path": "a.rs", "line_start": 5, "line_end": 9}),
        );
        assert_eq!(tool_tag_for_part(&read_grok).unwrap(), "<:read: a.rs:5-9:>");

        let grep = call(
            "Grep",
            serde_json::json!({"pattern": "export function", "glob": "*.ts"}),
        );
        assert_eq!(
            tool_tag_for_part(&grep).unwrap(),
            "<:grep: export function:>"
        );
        assert_eq!(part_body_text(&grep), "$ grep export function");

        let bash = call(
            "Bash",
            serde_json::json!({"command": "cd src && cargo build"}),
        );
        assert_eq!(
            tool_tag_for_part(&bash).unwrap(),
            "<:bash: cd src && cargo build:>"
        );
        assert_eq!(part_body_text(&bash), "$ cd src && cargo build");
    }

    #[test]
    fn write_and_edit_show_diff_previews() {
        let write = call(
            "Write",
            serde_json::json!({
                "file_path": "notes.md",
                "content": "line one\nline two",
            }),
        );
        assert_eq!(tool_tag_for_part(&write).unwrap(), "<:write: notes.md:>");
        assert_eq!(
            part_body_text(&write),
            "$ write notes.md\n```diff\n+ line one\n+ line two\n```"
        );

        let edit = call(
            "Edit",
            serde_json::json!({
                "file_path": "a.rs",
                "old_string": "old",
                "new_string": "new",
                "replace_all": false,
            }),
        );
        assert_eq!(tool_tag_for_part(&edit).unwrap(), "<:edit: a.rs:>");
        assert_eq!(
            part_body_text(&edit),
            "$ edit a.rs\n```diff\n- old\n+ new\n```"
        );
    }

    #[test]
    fn read_result_previews_as_code_block_and_others_as_output_lines() {
        let read_result = result("Read", serde_json::json!("fn main() {}\n"));
        assert_eq!(part_body_text(&read_result), "```\nfn main() {}\n```");

        let bash_result = result("Bash", serde_json::json!("ok\nwarning"));
        assert_eq!(part_body_text(&bash_result), "> ok\n> warning");

        let json_result = result("Grep", serde_json::json!([{"file": "a.rs", "line": 1}]));
        assert_eq!(
            part_body_text(&json_result),
            "> [\n>   {\n>     \"file\": \"a.rs\",\n>     \"line\": 1\n>   }\n> ]"
        );
    }

    #[test]
    fn opencode_camel_case_shapes_are_recognized() {
        let read = call(
            "read",
            serde_json::json!({"filePath": "src/main.rs", "offset": 11, "limit": 19}),
        );
        assert_eq!(
            tool_tag_for_part(&read).unwrap(),
            "<:read: src/main.rs:12-30:>"
        );

        let edit = call(
            "edit",
            serde_json::json!({
                "filePath": "a.rs",
                "oldString": "old",
                "newString": "new",
            }),
        );
        assert_eq!(tool_tag_for_part(&edit).unwrap(), "<:edit: a.rs:>");
        assert_eq!(
            part_body_text(&edit),
            "$ edit a.rs\n```diff\n- old\n+ new\n```"
        );

        let grep = call(
            "grep",
            serde_json::json!({"path": "a.rs", "pattern": "fn main"}),
        );
        assert_eq!(tool_tag_for_part(&grep).unwrap(), "<:grep: fn main:>");
    }

    #[test]
    fn grok_build_tool_names_are_recognized() {
        let read = call(
            "read_file",
            serde_json::json!({"target_file": "src/main.rs", "offset": 11, "limit": 19}),
        );
        assert_eq!(
            tool_tag_for_part(&read).unwrap(),
            "<:read: src/main.rs:12-30:>"
        );

        let bash = call(
            "run_terminal_command",
            serde_json::json!({"command": "cargo build"}),
        );
        assert_eq!(tool_tag_for_part(&bash).unwrap(), "<:bash: cargo build:>");
        assert_eq!(part_body_text(&bash), "$ cargo build");

        let edit = call(
            "search_replace",
            serde_json::json!({
                "file_path": "a.rs",
                "old_string": "old",
                "new_string": "new",
            }),
        );
        assert_eq!(tool_tag_for_part(&edit).unwrap(), "<:edit: a.rs:>");

        // use_tool dispatches MCP tools: the tag shows the target tool.
        let use_tool = call(
            "use_tool",
            serde_json::json!({
                "tool_name": "context7__resolve-library-id",
                "tool_input": {"query": "zed"},
            }),
        );
        assert_eq!(
            tool_tag_for_part(&use_tool).unwrap(),
            "<:context7: resolve-library-id:>"
        );
        assert_eq!(part_body_text(&use_tool), "$ context7: resolve-library-id");
    }

    #[test]
    fn codex_apply_patch_previews_as_diff() {
        let patch = call(
            "apply_patch",
            serde_json::json!("*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n"),
        );
        assert_eq!(tool_tag_for_part(&patch).unwrap(), "<:patch:>");
        assert_eq!(
            part_body_text(&patch),
            "$ patch\n```diff\n*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n```"
        );
    }

    #[test]
    fn unknown_tools_keep_the_generic_marker() {
        let unknown = call("wait", serde_json::json!({"cell_id": "1"}));
        assert_eq!(tool_tag_for_part(&unknown), None);
        assert!(part_body_text(&unknown).contains("<:tool:wait call:>"));
    }

    #[test]
    fn long_expressions_are_truncated() {
        let long_command = "x".repeat(100);
        let bash = call("Bash", serde_json::json!({"command": long_command}));
        let tag = tool_tag_for_part(&bash).unwrap();
        assert_eq!(
            tag.chars().count(),
            "<:bash: :>".chars().count() + MAX_EXPR + 1 // + "…"
        );
        assert!(tag.ends_with("…:>"));
    }
}
