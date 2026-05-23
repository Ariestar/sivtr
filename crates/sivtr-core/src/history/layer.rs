use serde::{Deserialize, Serialize};

/// Five-layer hierarchy for organizing terminal/agent I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerLevel {
    Workspace = 0,
    Source = 1,
    Session = 2,
    Dialogue = 3,
    Content = 4,
}

impl std::fmt::Display for LayerLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerLevel::Workspace => write!(f, "workspace"),
            LayerLevel::Source => write!(f, "source"),
            LayerLevel::Session => write!(f, "session"),
            LayerLevel::Dialogue => write!(f, "dialogue"),
            LayerLevel::Content => write!(f, "content"),
        }
    }
}

/// Content type for input entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputType {
    Command,
    Prompt,
    Message,
    ToolCall,
}

impl std::fmt::Display for InputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputType::Command => write!(f, "command"),
            InputType::Prompt => write!(f, "prompt"),
            InputType::Message => write!(f, "message"),
            InputType::ToolCall => write!(f, "tool_call"),
        }
    }
}

/// Content type for output entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputType {
    Text,
    Ansi,
    Message,
    ToolOutput,
    Error,
}

impl std::fmt::Display for OutputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputType::Text => write!(f, "text"),
            OutputType::Ansi => write!(f, "ansi"),
            OutputType::Message => write!(f, "message"),
            OutputType::ToolOutput => write!(f, "tool_output"),
            OutputType::Error => write!(f, "error"),
        }
    }
}

/// Full path through the layer hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerPath {
    pub workspace: String,
    pub source: String,
    pub session_id: String,
    pub dialogue_id: String,
}

/// An input entry stored in the database.
#[derive(Debug, Clone)]
pub struct InputEntry {
    pub id: i64,
    pub workspace: String,
    pub source: String,
    pub session_id: String,
    pub dialogue_id: String,
    pub content: String,
    pub content_type: String,
    pub timestamp: String,
}

/// An output entry stored in the database.
#[derive(Debug, Clone)]
pub struct OutputEntry {
    pub id: i64,
    pub workspace: String,
    pub source: String,
    pub session_id: String,
    pub dialogue_id: String,
    pub content: String,
    pub content_type: String,
    pub timestamp: String,
}

/// Workspace summary row.
#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub name: String,
    pub source_count: i64,
    pub session_count: i64,
    pub dialogue_count: i64,
    pub last_active: String,
}

/// Source summary row.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub name: String,
    pub session_count: i64,
    pub dialogue_count: i64,
    pub last_active: String,
}

/// Session summary row.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub dialogue_count: i64,
    pub first_at: String,
    pub last_at: String,
}

/// Dialogue summary row (one turn: input + output pair).
#[derive(Debug, Clone)]
pub struct DialogueInfo {
    pub id: String,
    pub input_count: i64,
    pub output_count: i64,
    pub first_at: String,
    pub last_at: String,
}
