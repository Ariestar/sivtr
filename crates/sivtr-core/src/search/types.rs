//! Search query enums shared by the CLI, MCP, eval, and remote protocol.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::record::WorkPartKind;

/// Whether search results address whole records or individual parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    #[default]
    Anchors,
    Parts,
}

/// Which field a match applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    #[default]
    Content,
    Title,
    Session,
    Input,
    Output,
    Command,
    All,
}

impl FromStr for Field {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "content" => Ok(Self::Content),
            "title" | "dialogue" | "dialog" => Ok(Self::Title),
            "session" => Ok(Self::Session),
            "input" => Ok(Self::Input),
            "output" => Ok(Self::Output),
            "command" | "cmd" => Ok(Self::Command),
            "all" => Ok(Self::All),
            _ => Err(format!(
                "unknown search field `{value}`; expected content, title, session, input, output, command, or all"
            )),
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Content => "content",
            Self::Title => "title",
            Self::Session => "session",
            Self::Input => "input",
            Self::Output => "output",
            Self::Command => "command",
            Self::All => "all",
        })
    }
}

/// How search results are ordered.
// kebab-case serde keeps the historical wire spelling (`duration-asc`, `exit-code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Duration,
    DurationAsc,
    ExitCode,
    ExitCodeAsc,
    /// BM25 relevance to the rank query.
    Relevance,
}

impl FromStr for Sort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "newest" | "latest" | "time" | "time-desc" => Ok(Self::Newest),
            "oldest" | "time-asc" => Ok(Self::Oldest),
            "duration" | "duration-desc" | "longest" => Ok(Self::Duration),
            "duration-asc" | "shortest" => Ok(Self::DurationAsc),
            "exit-code" | "exit_code" | "exit" | "exit-desc" => Ok(Self::ExitCode),
            "exit-code-asc" | "exit_code_asc" | "exit-asc" => Ok(Self::ExitCodeAsc),
            "relevance" | "re" => Ok(Self::Relevance),
            _ => Err(format!(
                "unknown search sort `{value}`; expected newest, oldest, duration, duration-asc, exit-code, exit-code-asc, or relevance"
            )),
        }
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::Duration => "duration",
            Self::DurationAsc => "duration-asc",
            Self::ExitCode => "exit-code",
            Self::ExitCodeAsc => "exit-code-asc",
            Self::Relevance => "relevance",
        })
    }
}

/// Part-kind filter for `--kind`. `Tool` matches both tool calls and results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Prompt,
    Command,
    User,
    Assistant,
    Tool,
    ToolCall,
    ToolResult,
    Skill,
    Thinking,
    Output,
    Error,
}

impl PartKind {
    pub fn matches(self, kind: WorkPartKind) -> bool {
        match self {
            Self::Prompt => kind == WorkPartKind::Prompt,
            Self::Command => kind == WorkPartKind::Command,
            Self::User => kind == WorkPartKind::User,
            Self::Assistant => kind == WorkPartKind::Assistant,
            Self::Tool => matches!(kind, WorkPartKind::ToolCall | WorkPartKind::ToolResult),
            Self::ToolCall => kind == WorkPartKind::ToolCall,
            Self::ToolResult => kind == WorkPartKind::ToolResult,
            Self::Skill => kind == WorkPartKind::Skill,
            Self::Thinking => kind == WorkPartKind::Thinking,
            Self::Output => kind == WorkPartKind::Output,
            Self::Error => kind == WorkPartKind::Error,
        }
    }
}

impl FromStr for PartKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().replace('-', "_").as_str() {
            "prompt" => Ok(Self::Prompt),
            "command" | "cmd" => Ok(Self::Command),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            "tool_call" | "call" => Ok(Self::ToolCall),
            "tool_result" | "result" => Ok(Self::ToolResult),
            "skill" => Ok(Self::Skill),
            "thinking" | "reason" | "reasoning" => Ok(Self::Thinking),
            "output" => Ok(Self::Output),
            "error" => Ok(Self::Error),
            _ => Err(format!(
                "unknown part kind `{value}`; expected prompt, command, user, assistant, tool, tool_call, tool_result, skill, thinking, output, or error"
            )),
        }
    }
}

impl fmt::Display for PartKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Prompt => "prompt",
            Self::Command => "command",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::Skill => "skill",
            Self::Thinking => "thinking",
            Self::Output => "output",
            Self::Error => "error",
        })
    }
}
