use super::refs::{WorkRef, WorkRefTarget};
use crate::ai::{
    format_blocks_with_text, select_blocks, AgentBlock, AgentBlockKind, AgentProvider,
    AgentSelection, AgentSession,
};
use crate::session::SessionEntry;
use crate::time::{derive_ended_at, derive_started_at, duration_between_ms, normalize_timestamp};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordTextMode {
    Combined,
    Input,
    Output,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordText {
    pub plain: String,
    pub ansi: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRecordCopyParts {
    pub input: RecordText,
    pub output: RecordText,
    pub block: RecordText,
    pub command: RecordText,
}

impl Default for RecordText {
    fn default() -> Self {
        Self::plain(String::new())
    }
}

impl RecordText {
    pub fn plain(plain: String) -> Self {
        Self { plain, ansi: None }
    }

    pub fn with_ansi(plain: String, ansi: String) -> Self {
        Self {
            plain,
            ansi: Some(ansi),
        }
    }

    pub fn rendered(&self, ansi: bool) -> &str {
        if ansi {
            self.ansi.as_deref().unwrap_or(&self.plain)
        } else {
            &self.plain
        }
    }
}

impl Default for WorkRecordCopyParts {
    fn default() -> Self {
        Self::from_block(RecordText::default())
    }
}

impl WorkRecordCopyParts {
    pub fn from_block(block: RecordText) -> Self {
        Self {
            input: block.clone(),
            output: block.clone(),
            block,
            command: RecordText::default(),
        }
    }
}

pub const RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRecordKind {
    TerminalCommand,
    ChatTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOutcome {
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkChannel {
    Terminal,
    Chat,
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
    pub canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub index: usize,
    pub work_ref: WorkRef,
}

impl WorkSessionRef {
    pub fn marker_ref(&self) -> String {
        match &self.work_ref {
            WorkRef::Terminal { session, .. } => format!("terminal/{session}"),
            WorkRef::Agent {
                provider, session, ..
            } => format!("{}/{session}", provider.command_name()),
        }
    }

    pub fn provider(&self) -> Option<AgentProvider> {
        self.work_ref.provider()
    }

    pub fn matches_id(&self, value: &str) -> bool {
        self.id == value || self.canonical_id.as_deref() == Some(value)
    }

    pub fn matches_record_ref(&self, reference: &WorkRef) -> bool {
        let reference = reference.record_ref();
        match (&self.work_ref, &reference) {
            (
                WorkRef::Terminal { record_index, .. },
                WorkRef::Terminal {
                    session,
                    record_index: wanted_index,
                    ..
                },
            ) => record_index == wanted_index && self.matches_id(session),
            (
                WorkRef::Agent {
                    provider,
                    turn_index,
                    ..
                },
                WorkRef::Agent {
                    provider: wanted_provider,
                    session,
                    turn_index: wanted_index,
                    ..
                },
            ) => {
                provider == wanted_provider
                    && turn_index == wanted_index
                    && self.matches_id(session)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl WorkTime {
    pub fn from_components(
        started_at: Option<String>,
        ended_at: Option<String>,
        duration_ms: Option<u64>,
    ) -> Self {
        let started_at = started_at.or_else(|| derive_started_at(ended_at.as_deref(), duration_ms));
        let ended_at = ended_at.or_else(|| derive_ended_at(started_at.as_deref(), duration_ms));
        let duration_ms =
            duration_ms.or_else(|| duration_between_ms(started_at.as_deref(), ended_at.as_deref()));

        Self {
            started_at,
            ended_at,
            duration_ms,
        }
    }

    pub fn primary_at(&self) -> Option<&str> {
        self.ended_at.as_deref().or(self.started_at.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkStatus {
    pub outcome: WorkOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPartIo {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPartKind {
    Prompt,
    Command,
    UserMessage,
    AssistantMessage,
    ToolCall,
    ToolOutput,
    Text,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkPart {
    pub io: WorkPartIo,
    pub kind: WorkPartKind,
    pub index_in_record: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ansi: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkText {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default)]
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
    pub work_ref: WorkRef,
    pub kind: WorkRecordKind,
    pub source: WorkSource,
    pub session: WorkSessionRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub time: WorkTime,
    pub status: WorkStatus,
    pub title: String,
    pub text: WorkText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<WorkPart>,
    pub payload: WorkPayload,
}

impl WorkRecord {
    pub fn terminal(entry: &SessionEntry, session_path: &Path, index: usize) -> Option<Self> {
        let command = entry.command.trim().to_string();
        let output = entry.output.trim().to_string();
        if command.is_empty() && output.is_empty() {
            return None;
        }

        let session_id = session_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("current")
            .to_string();
        let work_ref = WorkRef::terminal_record(session_id.clone(), index + 1);
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

        Some(Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id: work_ref.to_string(),
            work_ref,
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session_id.clone(),
                canonical_id: Some(session_id.clone()),
                path: Some(session_path.display().to_string()),
                index,
                work_ref: WorkRef::terminal_record(session_id, index + 1),
            },
            cwd: non_empty(entry.cwd.clone()),
            time: WorkTime::from_components(None, entry.ended_at.clone(), entry.duration_ms),
            status: WorkStatus {
                outcome,
                exit_code: entry.exit_code,
            },
            title,
            text: WorkText {
                input: non_empty(Some(command.clone())),
                output: non_empty(Some(output.clone())),
                combined: join_nonempty([command.as_str(), output.as_str()]),
            },
            parts: terminal_parts(entry, &command, &output),
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
        let chat_blocks = blocks
            .iter()
            .filter(|block| matches!(block.kind, AgentBlockKind::User | AgentBlockKind::Assistant))
            .cloned()
            .collect::<Vec<_>>();
        let messages = chat_blocks
            .iter()
            .map(|block| ChatMessage {
                role: block_role(block.kind).to_string(),
                content: compact_skill_blocks(block.text.trim()),
                timestamp: block.timestamp.as_deref().and_then(normalize_timestamp),
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
        if assistant.trim().is_empty() && !last_user_is_followed_only_by_tools(blocks) {
            return None;
        }

        let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
        let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
        let work_ref = WorkRef::agent_record(provider, session_ref_id.clone(), index + 1);
        let title = if user.trim().is_empty() {
            title_preview(&assistant)
        } else {
            title_preview(&user)
        };
        let started_at =
            first_timestamp(blocks).and_then(|timestamp| normalize_timestamp(&timestamp));
        let ended_at = last_timestamp(blocks).and_then(|timestamp| normalize_timestamp(&timestamp));

        Some(Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id: work_ref.to_string(),
            work_ref,
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some(provider.command_name().to_string()),
            },
            session: WorkSessionRef {
                id: session_ref_id,
                canonical_id,
                path: Some(session.path.display().to_string()),
                index,
                work_ref: WorkRef::agent_record(
                    provider,
                    agent_session_ref_id(session.id.as_deref(), &session.path),
                    index + 1,
                ),
            },
            cwd: non_empty(session.cwd.clone()),
            time: WorkTime::from_components(started_at, ended_at, None),
            status: WorkStatus {
                outcome: WorkOutcome::Unknown,
                exit_code: None,
            },
            title,
            text: WorkText {
                input: non_empty(Some(user.clone())),
                output: non_empty(Some(assistant.clone())),
                combined: format_blocks_with_compacted_skills(&chat_blocks),
            },
            parts: agent_parts(blocks),
            payload: WorkPayload::ChatTurn {
                user,
                assistant,
                messages,
            },
        })
    }

    pub fn selected_chat_records(
        provider: AgentProvider,
        session: &AgentSession,
        selection: AgentSelection,
    ) -> Vec<Self> {
        match selection {
            AgentSelection::LastTurn => Self::chat_turns(provider, session),
            AgentSelection::LastAssistant => {
                selected_block_records(provider, session, selection, AgentBlockKind::Assistant)
            }
            AgentSelection::LastUser => {
                selected_block_records(provider, session, selection, AgentBlockKind::User)
            }
            AgentSelection::LastTool => {
                selected_block_records(provider, session, selection, AgentBlockKind::ToolOutput)
            }
            AgentSelection::LastBlocks(_) | AgentSelection::All => {
                selected_group_record(provider, session, selection)
            }
        }
    }

    pub fn copy_text(&self, mode: RecordTextMode, include_prompt: bool) -> RecordText {
        match &self.payload {
            WorkPayload::TerminalCommand {
                prompt,
                command,
                output,
                prompt_ansi,
                output_ansi,
            } => {
                let context = TerminalTextContext {
                    prompt,
                    command,
                    output,
                    prompt_ansi: prompt_ansi.as_deref(),
                    output_ansi: output_ansi.as_deref(),
                    include_prompt,
                    prompt_override: None,
                };
                terminal_record_text(mode, context)
            }
            WorkPayload::ChatTurn { .. } => chat_record_text(self, mode),
        }
    }

    pub fn copy_text_with_prompt(
        &self,
        mode: RecordTextMode,
        include_prompt: bool,
        prompt_override: Option<&str>,
    ) -> RecordText {
        match &self.payload {
            WorkPayload::TerminalCommand {
                prompt,
                command,
                output,
                prompt_ansi,
                output_ansi,
            } => {
                let context = TerminalTextContext {
                    prompt,
                    command,
                    output,
                    prompt_ansi: prompt_ansi.as_deref(),
                    output_ansi: output_ansi.as_deref(),
                    include_prompt,
                    prompt_override,
                };
                terminal_record_text(mode, context)
            }
            WorkPayload::ChatTurn { .. } => chat_record_text(self, mode),
        }
    }

    pub fn copy_parts(&self, include_prompt: bool) -> WorkRecordCopyParts {
        self.copy_parts_with_prompt(include_prompt, None)
    }

    pub fn copy_parts_with_prompt(
        &self,
        include_prompt: bool,
        prompt_override: Option<&str>,
    ) -> WorkRecordCopyParts {
        WorkRecordCopyParts {
            input: self.copy_text_with_prompt(
                RecordTextMode::Input,
                include_prompt,
                prompt_override,
            ),
            output: self.copy_text_with_prompt(
                RecordTextMode::Output,
                include_prompt,
                prompt_override,
            ),
            block: self.copy_text_with_prompt(
                RecordTextMode::Combined,
                include_prompt,
                prompt_override,
            ),
            command: self.copy_text_with_prompt(RecordTextMode::Command, false, None),
        }
    }

    pub fn content_for_target(&self, target: WorkRefTarget) -> Option<&str> {
        match target {
            WorkRefTarget::Record => Some(self.text.combined.as_str()),
            WorkRefTarget::Line(line) => line
                .checked_sub(1)
                .and_then(|index| self.text.combined.lines().nth(index)),
            WorkRefTarget::Part { .. } => {
                self.part_for_target(target).map(|part| part.text.as_str())
            }
        }
    }

    pub fn part_for_target(&self, target: WorkRefTarget) -> Option<&WorkPart> {
        let (io, index) = match target {
            WorkRefTarget::Part { io, index } => (io, index),
            WorkRefTarget::Record | WorkRefTarget::Line(_) => return None,
        };
        self.parts
            .iter()
            .find(|part| part.io == io && part.index_in_record == index)
    }
}

struct TerminalTextContext<'a> {
    prompt: &'a str,
    command: &'a str,
    output: &'a str,
    prompt_ansi: Option<&'a str>,
    output_ansi: Option<&'a str>,
    include_prompt: bool,
    prompt_override: Option<&'a str>,
}

fn terminal_record_text(mode: RecordTextMode, context: TerminalTextContext<'_>) -> RecordText {
    match mode {
        RecordTextMode::Combined => {
            let input = terminal_input_text(
                context.prompt,
                context.command,
                context.prompt_ansi,
                context.include_prompt,
                context.prompt_override,
            );
            let output_ansi = context.output_ansi.unwrap_or(context.output).to_string();
            RecordText::with_ansi(
                join_terminal_input_output(&input.plain, context.output),
                join_terminal_input_output(input.rendered(true), &output_ansi),
            )
        }
        RecordTextMode::Input => terminal_input_text(
            context.prompt,
            context.command,
            context.prompt_ansi,
            context.include_prompt,
            context.prompt_override,
        ),
        RecordTextMode::Output => RecordText::with_ansi(
            context.output.to_string(),
            context.output_ansi.unwrap_or(context.output).to_string(),
        ),
        RecordTextMode::Command => RecordText::plain(context.command.to_string()),
    }
}

fn join_terminal_input_output(input: &str, output: &str) -> String {
    match (input.is_empty(), output.is_empty()) {
        (false, false) => format!("{input}\n{output}"),
        (false, true) => input.to_string(),
        (true, false) => output.to_string(),
        (true, true) => String::new(),
    }
}

fn terminal_input_text(
    prompt: &str,
    command: &str,
    prompt_ansi: Option<&str>,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> RecordText {
    if !include_prompt {
        return RecordText::plain(command.to_string());
    }

    let plain_prompt = render_input(prompt, command);
    let ansi_prompt = render_input(prompt_ansi.unwrap_or(prompt), command);
    let plain = match prompt_override {
        Some(prompt_override) if !command.is_empty() => {
            render_prompt_override(prompt_override, command)
        }
        Some(_) => plain_prompt.clone(),
        None => plain_prompt.clone(),
    };
    let ansi = match prompt_override {
        Some(prompt_override) if !command.is_empty() => {
            render_prompt_override(prompt_override, command)
        }
        Some(_) => plain_prompt,
        None => ansi_prompt,
    };
    RecordText::with_ansi(plain, ansi)
}

fn render_input(prompt: &str, command: &str) -> String {
    match (prompt.trim_end().is_empty(), command.is_empty()) {
        (false, false) => render_prompt_override(prompt, command),
        (false, true) => prompt.trim_end_matches(['\r', '\n']).to_string(),
        (true, false) => command.to_string(),
        (true, true) => String::new(),
    }
}

fn render_prompt_override(prompt: &str, command: &str) -> String {
    let prompt = prompt.trim_end_matches(['\r', '\n']);
    if prompt.is_empty() {
        return command.to_string();
    }

    if prompt.ends_with(' ') || prompt.ends_with('\t') {
        format!("{prompt}{command}")
    } else {
        format!("{prompt} {command}")
    }
}

fn chat_record_text(record: &WorkRecord, mode: RecordTextMode) -> RecordText {
    let text = match mode {
        RecordTextMode::Combined => join_nonempty([
            record.text.input.as_deref().unwrap_or(""),
            record.text.output.as_deref().unwrap_or(""),
        ]),
        RecordTextMode::Input => record.text.input.clone().unwrap_or_default(),
        RecordTextMode::Output => record.text.output.clone().unwrap_or_default(),
        RecordTextMode::Command => String::new(),
    };
    RecordText::plain(text)
}

fn selected_block_records(
    provider: AgentProvider,
    session: &AgentSession,
    selection: AgentSelection,
    kind: AgentBlockKind,
) -> Vec<WorkRecord> {
    select_blocks(session, selection)
        .into_iter()
        .enumerate()
        .filter(|(_, block)| !block.text.trim().is_empty())
        .map(|(index, block)| selected_block_record(provider, session, kind, index, block))
        .collect()
}

fn selected_block_record(
    provider: AgentProvider,
    session: &AgentSession,
    kind: AgentBlockKind,
    index: usize,
    block: AgentBlock,
) -> WorkRecord {
    let text = compact_skill_blocks(block.text.trim());
    let title = title_preview(&text);
    let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
    let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
    let work_ref = WorkRef::agent_record(provider, session_ref_id.clone(), index + 1);
    let block_timestamp = block.timestamp.as_deref().and_then(normalize_timestamp);
    let (input, output) = match kind {
        AgentBlockKind::User => (Some(text.clone()), None),
        AgentBlockKind::Assistant | AgentBlockKind::ToolOutput => (None, Some(text.clone())),
        AgentBlockKind::ToolCall => (None, None),
    };

    WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        id: work_ref.to_string(),
        work_ref,
        kind: WorkRecordKind::ChatTurn,
        source: WorkSource {
            channel: WorkChannel::Chat,
            provider: Some(provider.command_name().to_string()),
        },
        session: WorkSessionRef {
            id: session_ref_id,
            canonical_id,
            path: Some(session.path.display().to_string()),
            index,
            work_ref: WorkRef::agent_record(
                provider,
                agent_session_ref_id(session.id.as_deref(), &session.path),
                index + 1,
            ),
        },
        cwd: non_empty(session.cwd.clone()),
        time: WorkTime::from_components(block_timestamp.clone(), None, None),
        status: WorkStatus {
            outcome: WorkOutcome::Unknown,
            exit_code: None,
        },
        title,
        text: WorkText {
            input: input.clone(),
            output: output.clone(),
            combined: text.clone(),
        },
        parts: agent_parts(std::slice::from_ref(&block)),
        payload: WorkPayload::ChatTurn {
            user: match kind {
                AgentBlockKind::User => text.clone(),
                _ => String::new(),
            },
            assistant: match kind {
                AgentBlockKind::User => String::new(),
                _ => text.clone(),
            },
            messages: vec![ChatMessage {
                role: block_role(block.kind).to_string(),
                content: text.clone(),
                timestamp: block_timestamp,
            }],
        },
    }
}

fn selected_group_record(
    provider: AgentProvider,
    session: &AgentSession,
    selection: AgentSelection,
) -> Vec<WorkRecord> {
    let blocks = select_blocks(session, selection);
    let combined = format_blocks_with_compacted_skills(&blocks);
    if combined.trim().is_empty() {
        return Vec::new();
    }

    let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
    let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
    let work_ref = WorkRef::agent_record(provider, session_ref_id.clone(), 1);
    let started_at = first_timestamp(&blocks).and_then(|timestamp| normalize_timestamp(&timestamp));
    let ended_at = last_timestamp(&blocks).and_then(|timestamp| normalize_timestamp(&timestamp));
    let parts = agent_parts(&blocks);
    vec![WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        id: work_ref.to_string(),
        work_ref,
        kind: WorkRecordKind::ChatTurn,
        source: WorkSource {
            channel: WorkChannel::Chat,
            provider: Some(provider.command_name().to_string()),
        },
        session: WorkSessionRef {
            id: session_ref_id,
            canonical_id,
            path: Some(session.path.display().to_string()),
            index: 0,
            work_ref: WorkRef::agent_record(
                provider,
                agent_session_ref_id(session.id.as_deref(), &session.path),
                1,
            ),
        },
        cwd: non_empty(session.cwd.clone()),
        time: WorkTime::from_components(started_at, ended_at, None),
        status: WorkStatus {
            outcome: WorkOutcome::Unknown,
            exit_code: None,
        },
        title: title_preview(&combined),
        text: WorkText {
            input: None,
            output: Some(combined.clone()),
            combined: combined.clone(),
        },
        parts,
        payload: WorkPayload::ChatTurn {
            user: String::new(),
            assistant: combined,
            messages: blocks
                .into_iter()
                .map(|block| ChatMessage {
                    role: block_role(block.kind).to_string(),
                    content: compact_skill_blocks(block.text.trim()),
                    timestamp: block.timestamp.as_deref().and_then(normalize_timestamp),
                })
                .filter(|message| !message.content.is_empty())
                .collect(),
        },
    }]
}

pub fn chat_turn_ranges(blocks: &[AgentBlock]) -> Vec<(usize, usize)> {
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
        } else if let Some(previous_user_only_turn) =
            user_only_turn_before_trailing_tools(blocks, start)
        {
            ranges.push(previous_user_only_turn);
        }
    }

    ranges
}

fn user_only_turn_before_trailing_tools(
    blocks: &[AgentBlock],
    start: usize,
) -> Option<(usize, usize)> {
    blocks
        .get(start + 1..)?
        .iter()
        .all(|block| {
            matches!(
                block.kind,
                AgentBlockKind::ToolCall | AgentBlockKind::ToolOutput
            )
        })
        .then_some((start, blocks.len()))
}

pub fn is_real_user_block(block: &AgentBlock) -> bool {
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

fn agent_session_canonical_id(id: Option<&str>, path: &Path) -> Option<String> {
    id.filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
                .filter(|id| !id.trim().is_empty())
        })
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

fn last_timestamp(blocks: &[AgentBlock]) -> Option<String> {
    blocks
        .iter()
        .rev()
        .find_map(|block| block.timestamp.clone())
}

fn last_user_is_followed_only_by_tools(blocks: &[AgentBlock]) -> bool {
    let Some(user_idx) = blocks
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::User)
    else {
        return false;
    };

    blocks[user_idx + 1..].iter().all(|block| {
        matches!(
            block.kind,
            AgentBlockKind::ToolCall | AgentBlockKind::ToolOutput
        )
    })
}

fn format_blocks_with_compacted_skills(blocks: &[AgentBlock]) -> String {
    format_blocks_with_text(blocks, |block| compact_skill_blocks(block.text.trim()))
}

fn compact_skill_blocks(text: &str) -> String {
    let mut output = String::new();
    let mut rest = text;

    while let Some(start) = rest.find("<skill ") {
        output.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let Some(open_end) = candidate.find('>') else {
            output.push_str(candidate);
            return trim_preserving_inner_whitespace(&output);
        };
        let opening = &candidate[..=open_end];
        let Some(name) = skill_attribute(opening, "name") else {
            output.push_str(&candidate[..=open_end]);
            rest = &candidate[open_end + 1..];
            continue;
        };
        let after_open = &candidate[open_end + 1..];
        let Some(close_start) = after_open.find("</skill>") else {
            output.push_str(candidate);
            return trim_preserving_inner_whitespace(&output);
        };

        output.push_str(&format!("[skill:{name}]"));
        rest = &after_open[close_start + "</skill>".len()..];
    }

    output.push_str(rest);
    trim_preserving_inner_whitespace(&output)
}

fn skill_attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

fn trim_preserving_inner_whitespace(text: &str) -> String {
    text.trim().to_string()
}

fn title_preview(text: &str) -> String {
    preview_from_lines(
        text.lines()
            .filter(|line| !is_skill_marker_line(line.trim())),
    )
}

fn preview(text: &str) -> String {
    preview_from_lines(text.lines())
}

fn preview_from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    lines
        .into_iter()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## "))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

fn is_skill_marker_line(line: &str) -> bool {
    line.starts_with("[skill:") && line.ends_with(']')
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

fn terminal_parts(entry: &SessionEntry, command: &str, output: &str) -> Vec<WorkPart> {
    let mut parts = Vec::new();
    let mut input_count = 0;
    let mut output_count = 0;
    let prompt = entry.prompt.trim_end_matches(['\r', '\n']);
    if !prompt.trim().is_empty() {
        input_count += 1;
        parts.push(WorkPart {
            io: WorkPartIo::Input,
            kind: WorkPartKind::Prompt,
            index_in_record: input_count,
            occurred_at: entry.ended_at.clone(),
            label: None,
            text: prompt.to_string(),
            ansi: entry.prompt_ansi.clone(),
        });
    }
    if !command.is_empty() {
        input_count += 1;
        parts.push(WorkPart {
            io: WorkPartIo::Input,
            kind: WorkPartKind::Command,
            index_in_record: input_count,
            occurred_at: entry.ended_at.clone(),
            label: None,
            text: command.to_string(),
            ansi: None,
        });
    }
    if !output.is_empty() {
        output_count += 1;
        parts.push(WorkPart {
            io: WorkPartIo::Output,
            kind: WorkPartKind::Text,
            index_in_record: output_count,
            occurred_at: entry.ended_at.clone(),
            label: None,
            text: output.to_string(),
            ansi: entry.output_ansi.clone(),
        });
    }
    parts
}

fn agent_parts(blocks: &[AgentBlock]) -> Vec<WorkPart> {
    let mut parts = Vec::new();
    let mut input_count = 0;
    let mut output_count = 0;
    for block in blocks {
        let text = block.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let (io, kind) = match block.kind {
            AgentBlockKind::User => (WorkPartIo::Input, WorkPartKind::UserMessage),
            AgentBlockKind::Assistant => (WorkPartIo::Output, WorkPartKind::AssistantMessage),
            AgentBlockKind::ToolCall => (WorkPartIo::Input, WorkPartKind::ToolCall),
            AgentBlockKind::ToolOutput => (WorkPartIo::Output, WorkPartKind::ToolOutput),
        };
        let index_in_record = match io {
            WorkPartIo::Input => {
                input_count += 1;
                input_count
            }
            WorkPartIo::Output => {
                output_count += 1;
                output_count
            }
        };
        parts.push(WorkPart {
            io,
            kind,
            index_in_record,
            occurred_at: block.timestamp.clone(),
            label: block.label.clone(),
            text,
            ansi: None,
        });
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_timestamp;
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
        assert_eq!(record.cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(
            record.time.ended_at.as_deref().and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:00:00Z")
        );
        assert_eq!(
            record.time.started_at.as_deref().and_then(parse_timestamp),
            parse_timestamp("2026-05-23T11:59:59.958Z")
        );
        assert_eq!(record.time.duration_ms, Some(42));
        assert_eq!(record.status.outcome, WorkOutcome::Failure);
        assert_eq!(record.status.exit_code, Some(101));
        assert_eq!(record.text.input.as_deref(), Some("cargo test"));
        assert_eq!(record.text.output.as_deref(), Some("failed"));
    }

    #[test]
    fn chat_turn_records_keep_interrupted_user_tool_turn() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    text: "fix latest terminal error".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolCall,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: Some("bash".to_string()),
                    text: "{\"command\":\"cargo test\"}".to_string(),
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolOutput,
                    timestamp: Some("2026-05-23T12:03:00Z".to_string()),
                    label: Some("bash".to_string()),
                    text: "failed".to_string(),
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Pi, &session);

        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].text.input.as_deref(),
            Some("fix latest terminal error")
        );
        assert_eq!(records[0].text.output, None);
        assert_eq!(
            records[0]
                .time
                .started_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:01:00Z")
        );
        assert_eq!(
            records[0]
                .time
                .ended_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:03:00Z")
        );
        assert_eq!(records[0].time.duration_ms, Some(120_000));
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
        assert_eq!(records[0].work_ref.provider(), Some(AgentProvider::Pi));
        assert_eq!(records[0].text.input.as_deref(), Some("implement this"));
        assert_eq!(records[0].text.output.as_deref(), Some("done"));
        assert_eq!(
            records[0]
                .time
                .started_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:01:00Z")
        );
        assert_eq!(
            records[0]
                .time
                .ended_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:02:00Z")
        );
        assert_eq!(records[0].time.duration_ms, Some(60_000));
    }

    #[test]
    fn chat_turn_records_compact_skill_blocks() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    text: "<skill name=\"sivtr-memory\" location=\"C:\\x\\SKILL.md\">\nlong instructions\n</skill>\n\nreal task".to_string(),
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
        assert_eq!(records[0].title, "real task");
        assert_eq!(
            records[0].text.input.as_deref(),
            Some("[skill:sivtr-memory]\n\nreal task")
        );
        match &records[0].payload {
            WorkPayload::ChatTurn {
                user,
                assistant,
                messages,
            } => {
                assert_eq!(user, "[skill:sivtr-memory]\n\nreal task");
                assert_eq!(assistant, "done");
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].role, "user");
                assert_eq!(messages[0].content, "[skill:sivtr-memory]\n\nreal task");
                assert_eq!(
                    messages[0].timestamp.as_deref().and_then(parse_timestamp),
                    parse_timestamp("2026-05-23T12:01:00Z")
                );
                assert_eq!(messages[1].role, "assistant");
                assert_eq!(messages[1].content, "done");
                assert_eq!(
                    messages[1].timestamp.as_deref().and_then(parse_timestamp),
                    parse_timestamp("2026-05-23T12:02:00Z")
                );
            }
            _ => panic!("expected chat payload"),
        }
    }
}
