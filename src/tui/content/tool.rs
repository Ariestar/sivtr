//! Shape-driven tool display for the content pane.
//!
//! Tool calls are classified by category — command (`$ cmd`), read (`$ read
//! path` + code preview), search (`$ grep pattern`), edit (diff preview),
//! web (`$ webfetch url`) — and get per-tool tags (`<:read: src/main.rs:12-30:>`)
//! instead of the generic `<:tool:Name call:>` marker. The formatter keys off
//! the normalized `tool` name and the `input` / `output` JSON, so every
//! provider (claude, grok, codex, opencode, …) flows through one code path —
//! display only; evidence export keeps its original markers.

use serde_json::Value;
use sivtr_core::record::{WorkPart, WorkPartData};

/// Long input expressions are truncated to fit a tag line.
const MAX_EXPR: usize = 40;

/// Tool category: drives how a tool call is displayed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolCategory {
    /// Shell-like: `$ command` is the whole instruction (`bash`, `exec`,
    /// `run_terminal_command`, `shell_command`).
    Command,
    /// File read: `$ read path:lines`, code preview from the result.
    Read,
    /// Text search: `$ grep pattern`.
    Search,
    /// File modification: diff preview from the input (`edit`, `write`,
    /// `apply_patch`).
    Edit,
    /// Remote fetch: `$ webfetch url`.
    Web,
}

/// Display spec of a known tool: its category and canonical tag name.
struct ToolSpec {
    category: ToolCategory,
    name: &'static str,
}

/// Known tools by provider name (claude `Read`, opencode `read`, codex
/// `apply_patch`/`exec`, grok build `read_file`/`run_terminal_command`/
/// `search_replace`); `None` for unknown tools that keep the generic marker.
fn tool_spec(tool: &str) -> Option<&'static ToolSpec> {
    use ToolCategory::*;
    Some(match tool.to_ascii_lowercase().as_str() {
        "bash"
        | "shell"
        | "run_terminal_command"
        | "shell_command"
        | "run_command"
        | "run_command_or_subagent" => &ToolSpec {
            category: Command,
            name: "bash",
        },
        "exec" => &ToolSpec {
            category: Command,
            name: "exec",
        },
        "read" | "read_file" => &ToolSpec {
            category: Read,
            name: "read",
        },
        "grep" | "search_files" => &ToolSpec {
            category: Search,
            name: "grep",
        },
        "edit" | "search_replace" => &ToolSpec {
            category: Edit,
            name: "edit",
        },
        "write" => &ToolSpec {
            category: Edit,
            name: "write",
        },
        "apply_patch" | "patch" => &ToolSpec {
            category: Edit,
            name: "patch",
        },
        "notebookedit" | "notebook_edit" => &ToolSpec {
            category: Edit,
            name: "notebook-edit",
        },
        "webfetch" | "web_fetch" => &ToolSpec {
            category: Web,
            name: "webfetch",
        },
        "websearch" | "web_search" => &ToolSpec {
            category: Web,
            name: "websearch",
        },
        _ => return None,
    })
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
    tool_spec(tool)
        .map(|spec| spec.name.to_string())
        .unwrap_or_else(|| tool.to_ascii_lowercase())
}

/// Whether the tool gets the new per-tool rendering (known names + MCP).
fn is_known_tool(tool: &str) -> bool {
    tool_spec(tool).is_some() || tool.starts_with("mcp__") || is_use_tool(tool)
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
/// marker. Long expressions are truncated to fit the tag line.
pub(crate) fn tool_tag(tool: &str, value: &Value) -> Option<String> {
    if tool.starts_with("mcp__") || is_use_tool(tool) {
        return Some(format!("<:{}:>", tool_call_name(tool, value)));
    }
    let spec = tool_spec(tool)?;
    if spec.name == "patch" {
        // apply_patch always carries a raw patch string; no short expr.
        return Some("<:patch:>".to_string());
    }
    let expr = tool_input_expr(tool, value)?;
    Some(format!("<:{}: {}:>", spec.name, truncate(expr)))
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
        WorkPartData::ToolResult {
            tool,
            output,
            start_line,
            ..
        } => {
            let tool = tool.as_deref().unwrap_or_default();
            if is_known_tool(tool) {
                tool_result_text(tool, output, *start_line)
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
/// Full text; callers truncate for tag lines.
fn tool_input_expr(tool: &str, input: &Value) -> Option<String> {
    let spec = tool_spec(tool)?;
    let expr = match spec.category {
        ToolCategory::Command => match input {
            Value::String(script) => script.trim().to_string(),
            _ => input
                .as_object()?
                .get("command")?
                .as_str()?
                .trim()
                .to_string(),
        },
        ToolCategory::Read => {
            let obj = input.as_object()?;
            let path = path_field(obj)?;
            match line_range(obj) {
                Some(range) => format!("{path}:{range}"),
                None => path.to_string(),
            }
        }
        ToolCategory::Search => input
            .as_object()?
            .get("pattern")?
            .as_str()?
            .trim()
            .to_string(),
        ToolCategory::Edit => match spec.name {
            "patch" => return None,
            _ => path_field(input.as_object()?)?.to_string(),
        },
        ToolCategory::Web => input
            .as_object()?
            .get("url")
            .or_else(|| input.as_object()?.get("query"))?
            .as_str()?
            .trim()
            .to_string(),
    };
    if expr.is_empty() {
        return None;
    }
    Some(expr)
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
/// for edit/write (which carry content in the input).
pub(crate) fn tool_call_text(tool: &str, input: &Value) -> String {
    let name = tool_call_name(tool, input);
    let expr = tool_input_expr(tool, input);
    let line = if tool_spec(tool).is_some_and(|spec| spec.category == ToolCategory::Command) {
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
/// for edit old/new strings, the raw unified diff for apply_patch, inside a
/// ```diff fence the pane colors.
fn diff_preview(tool: &str, input: &Value) -> Option<String> {
    let spec = tool_spec(tool)?;
    if spec.category != ToolCategory::Edit {
        return None;
    }
    let diff = match spec.name {
        "patch" => {
            let Value::String(patch) = input else {
                return None;
            };
            let patch = patch.trim_end();
            if patch.is_empty() {
                return None;
            }
            patch.to_string()
        }
        "write" => {
            let content = input.as_object()?.get("content")?.as_str()?;
            content
                .lines()
                .map(|line| format!("+ {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => {
            // edit / notebook-edit: old_string → `-`, new_string → `+`.
            let obj = input.as_object()?;
            let mut diff = Vec::new();
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
            if diff.is_empty() {
                return None;
            }
            diff.join("\n")
        }
    };
    Some(format!("```diff\n{diff}\n```"))
}

/// Expanded body of a tool result: `>` output lines, or a fenced block for
/// read (the file content preview) and every search tool. Text matches fence
/// as a structured ` ```grep ` block (summary, paths, line numbers); JSON
/// results (e.g. opencode's `search_files`) keep their data shape as a
/// ` ```json ` block. Provider envelopes (grok's `<workspace_result …>`,
/// `N→` line gutters) are already stripped by the parser, whose `start_line`
/// metadata shifts the read gutter to real file lines.
pub(crate) fn tool_result_text(tool: &str, output: &Value, start_line: Option<u64>) -> String {
    let text = match output {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(output).unwrap_or_default(),
    };
    let Some(spec) = tool_spec(tool) else {
        return output_lines(&text);
    };
    match spec.category {
        ToolCategory::Read => match start_line {
            Some(start) => format!("```{start}\n{}\n```", text.trim_end()),
            None => format!("```\n{}\n```", text.trim_end()),
        },
        ToolCategory::Search => match output {
            Value::String(_) => format!("```grep\n{}\n```", text.trim_end()),
            _ => format!("```json\n{}\n```", text.trim_end()),
        },
        _ => output_lines(&text),
    }
}

fn output_lines(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
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
        result_with_line(tool, output, None)
    }

    fn result_with_line(
        tool: &str,
        output: serde_json::Value,
        start_line: Option<u64>,
    ) -> WorkPart {
        WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: None,
                tool: Some(tool.to_string()),
                output,
                start_line,
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

        // A numbered read result shifts the code gutter to the file's line.
        let numbered = result_with_line("Read", serde_json::json!("line one\nline two"), Some(775));
        assert_eq!(part_body_text(&numbered), "```775\nline one\nline two\n```");

        let bash_result = result("Bash", serde_json::json!("ok\nwarning"));
        assert_eq!(part_body_text(&bash_result), "> ok\n> warning");

        // Text grep results fence as a structured search block; JSON results
        // (opencode's search_files) keep their data shape as a JSON block.
        let text_match = result(
            "Grep",
            serde_json::json!("Found 2 matching lines\nD:\\Coding\\AGENTS.md\n31:- rule\n"),
        );
        assert_eq!(
            part_body_text(&text_match),
            "```grep\nFound 2 matching lines\nD:\\Coding\\AGENTS.md\n31:- rule\n```"
        );
        let json_result = result("Grep", serde_json::json!([{"file": "a.rs", "line": 1}]));
        assert_eq!(
            part_body_text(&json_result),
            "```json\n[\n  {\n    \"file\": \"a.rs\",\n    \"line\": 1\n  }\n]\n```"
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
    fn exec_is_a_command_like_bash() {
        let script =
            "const skill = await tools.shell_command({command:\"cargo build\"});\ntext(skill);";
        let exec = call("exec", serde_json::json!(script));
        // Command category: `$` line with the command, tag with the tool
        // name, long scripts truncated to fit the tag line.
        let tag = tool_tag_for_part(&exec).unwrap();
        assert!(tag.starts_with("<:exec: const skill = await tools.shell_command("));
        assert!(tag.ends_with("…:>"));
        assert_eq!(part_body_text(&exec), format!("$ {script}"));

        let shell = call(
            "shell_command",
            serde_json::json!({"command": "cargo test"}),
        );
        assert_eq!(tool_tag_for_part(&shell).unwrap(), "<:bash: cargo test:>");
        assert_eq!(part_body_text(&shell), "$ cargo test");
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
