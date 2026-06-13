use super::refs::{WorkRef, WorkRefTarget};
use crate::ai::{
    format_blocks_with_text, select_blocks, AgentBlock, AgentBlockKind, AgentProvider,
    AgentSelection, AgentSession,
};
use crate::session::SessionEntry;
use crate::time::{derive_ended_at, derive_started_at, duration_between_ms, normalize_timestamp};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRecordKind {
    TerminalCommand,
    ChatTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOutcome {
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkChannel {
    Terminal,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSource {
    pub channel: WorkChannel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSessionRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl WorkSessionRef {
    pub fn matches_id(&self, value: &str) -> bool {
        self.id == value || self.canonical_id.as_deref() == Some(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStatus {
    pub outcome: WorkOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPartIo {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkPart {
    pub io: WorkPartIo,
    pub kind: WorkPartKind,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRecord {
    pub schema_version: u32,
    pub work_ref: WorkRef,
    pub kind: WorkRecordKind,
    pub source: WorkSource,
    pub session: WorkSessionRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub time: WorkTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkStatus>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<WorkPart>,
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
            },
            cwd: non_empty(entry.cwd.clone().unwrap_or_default()),
            time: WorkTime::from_components(None, entry.ended_at.clone(), entry.duration_ms),
            status: Some(WorkStatus {
                outcome,
                exit_code: entry.exit_code,
            }),
            title,
            parts: terminal_parts(entry, &command, &output),
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
        let parts = agent_parts(blocks);
        let user = join_part_text(
            parts
                .iter()
                .filter(|part| matches!(part.kind, WorkPartKind::UserMessage)),
        );
        let assistant = join_part_text(
            parts
                .iter()
                .filter(|part| matches!(part.kind, WorkPartKind::AssistantMessage)),
        );
        if user.trim().is_empty() && assistant.trim().is_empty() {
            return None;
        }
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
            },
            cwd: non_empty(session.cwd.clone().unwrap_or_default()),
            time: WorkTime::from_components(started_at, ended_at, None),
            status: None,
            title,
            parts,
        })
    }

    pub fn selected_chat_records(
        provider: AgentProvider,
        session: &AgentSession,
        selection: AgentSelection,
    ) -> Vec<Self> {
        match selection {
            AgentSelection::LastTurn => Self::chat_turns(provider, session),
            AgentSelection::LastAssistant => selected_block_records(provider, session, selection),
            AgentSelection::LastUser => selected_block_records(provider, session, selection),
            AgentSelection::LastTool => selected_block_records(provider, session, selection),
            AgentSelection::LastBlocks(_) | AgentSelection::All => {
                selected_group_record(provider, session, selection)
            }
        }
    }

    pub fn copy_text(&self, mode: RecordTextMode, include_prompt: bool) -> RecordText {
        self.copy_text_with_prompt(mode, include_prompt, None)
    }

    pub fn copy_text_with_prompt(
        &self,
        mode: RecordTextMode,
        include_prompt: bool,
        prompt_override: Option<&str>,
    ) -> RecordText {
        match self.kind {
            WorkRecordKind::TerminalCommand => terminal_record_text(
                mode,
                TerminalTextContext {
                    parts: &self.parts,
                    include_prompt,
                    prompt_override,
                },
            ),
            WorkRecordKind::ChatTurn => chat_record_text(self, mode),
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

    pub fn combined_text(&self) -> String {
        let mut text = String::new();
        self.write_combined_text(&mut text);
        text
    }

    pub fn input_text(&self) -> Option<String> {
        match self.kind {
            WorkRecordKind::TerminalCommand => non_empty(join_part_text(
                self.parts
                    .iter()
                    .filter(|part| matches!(part.kind, WorkPartKind::Command)),
            )),
            WorkRecordKind::ChatTurn => non_empty(join_part_text(
                self.parts
                    .iter()
                    .filter(|part| matches!(part.kind, WorkPartKind::UserMessage)),
            )),
        }
    }

    pub fn output_text(&self) -> Option<String> {
        match self.kind {
            WorkRecordKind::TerminalCommand => non_empty(join_part_text(
                self.parts
                    .iter()
                    .filter(|part| matches!(part.kind, WorkPartKind::Text)),
            )),
            WorkRecordKind::ChatTurn => {
                non_empty(join_part_text(self.parts.iter().filter(|part| {
                    matches!(part.kind, WorkPartKind::AssistantMessage)
                })))
            }
        }
    }

    pub fn content_for_target(&self, target: WorkRefTarget) -> Option<String> {
        match target {
            WorkRefTarget::Record => Some(self.combined_text()).filter(|text| !text.is_empty()),
            WorkRefTarget::Line(line) => self.line_text(line),
            WorkRefTarget::Part { .. } => {
                self.part_for_target(target).map(|part| part.text.clone())
            }
        }
    }

    fn write_combined_text(&self, text: &mut String) {
        match self.kind {
            WorkRecordKind::TerminalCommand => {
                for part in self
                    .parts
                    .iter()
                    .filter(|part| matches!(part.kind, WorkPartKind::Command | WorkPartKind::Text))
                {
                    append_text_segment(text, &part.text);
                }
            }
            WorkRecordKind::ChatTurn => {
                for part in self.parts.iter().filter(|part| {
                    matches!(
                        part.kind,
                        WorkPartKind::UserMessage | WorkPartKind::AssistantMessage
                    )
                }) {
                    append_text_segment(text, &part.text);
                }
            }
        }
    }

    fn line_text(&self, line: usize) -> Option<String> {
        let target_index = line.checked_sub(1)?;
        self.combined_text()
            .lines()
            .filter(|line| !line.is_empty())
            .nth(target_index)
            .map(str::to_string)
    }

    pub fn part_for_target(&self, target: WorkRefTarget) -> Option<&WorkPart> {
        let (io, index) = match target {
            WorkRefTarget::Part { io, index } => (io, index),
            WorkRefTarget::Record | WorkRefTarget::Line(_) => return None,
        };
        self.parts
            .iter()
            .find(|part| part.io == io && part.index == index)
    }
}

struct TerminalTextContext<'a> {
    parts: &'a [WorkPart],
    include_prompt: bool,
    prompt_override: Option<&'a str>,
}

fn terminal_record_text(mode: RecordTextMode, context: TerminalTextContext<'_>) -> RecordText {
    let prompt_part = first_part(context.parts, WorkPartKind::Prompt);
    let command_part = first_part(context.parts, WorkPartKind::Command);
    let output_part = first_part(context.parts, WorkPartKind::Text);
    let prompt = prompt_part.map(|part| part.text.as_str()).unwrap_or("");
    let command = command_part.map(|part| part.text.as_str()).unwrap_or("");
    let output = output_part.map(|part| part.text.as_str()).unwrap_or("");
    let prompt_ansi = prompt_part.and_then(|part| part.ansi.as_deref());
    let output_ansi = output_part.and_then(|part| part.ansi.as_deref());

    match mode {
        RecordTextMode::Combined => {
            let input = terminal_input_text(
                prompt,
                command,
                prompt_ansi,
                context.include_prompt,
                context.prompt_override,
            );
            let output_ansi = output_ansi.unwrap_or(output).to_string();
            RecordText::with_ansi(
                join_terminal_input_output(&input.plain, output),
                join_terminal_input_output(input.rendered(true), &output_ansi),
            )
        }
        RecordTextMode::Input => terminal_input_text(
            prompt,
            command,
            prompt_ansi,
            context.include_prompt,
            context.prompt_override,
        ),
        RecordTextMode::Output => RecordText::with_ansi(
            output.to_string(),
            output_ansi.unwrap_or(output).to_string(),
        ),
        RecordTextMode::Command => RecordText::plain(command.to_string()),
    }
}

fn first_part(parts: &[WorkPart], kind: WorkPartKind) -> Option<&WorkPart> {
    parts.iter().find(|part| part.kind == kind)
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
        RecordTextMode::Combined => record.combined_text(),
        RecordTextMode::Input => record.input_text().unwrap_or_default(),
        RecordTextMode::Output => record.output_text().unwrap_or_default(),
        RecordTextMode::Command => String::new(),
    };
    RecordText::plain(text)
}

fn selected_block_records(
    provider: AgentProvider,
    session: &AgentSession,
    selection: AgentSelection,
) -> Vec<WorkRecord> {
    select_blocks(session, selection)
        .into_iter()
        .enumerate()
        .filter(|(_, block)| !block.text.trim().is_empty())
        .map(|(index, block)| selected_block_record(provider, session, index, block))
        .collect()
}

fn selected_block_record(
    provider: AgentProvider,
    session: &AgentSession,
    index: usize,
    block: AgentBlock,
) -> WorkRecord {
    let text = compact_skill_blocks(block.text.trim());
    let title = title_preview(&text);
    let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
    let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
    let work_ref = WorkRef::agent_record(provider, session_ref_id.clone(), index + 1);
    let block_timestamp = block.timestamp.as_deref().and_then(normalize_timestamp);
    WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
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
        },
        cwd: non_empty(session.cwd.clone().unwrap_or_default()),
        time: WorkTime::from_components(block_timestamp.clone(), None, None),
        status: None,
        title,
        parts: agent_parts(std::slice::from_ref(&block)),
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
        },
        cwd: non_empty(session.cwd.clone().unwrap_or_default()),
        time: WorkTime::from_components(started_at, ended_at, None),
        status: None,
        title: title_preview(&combined),
        parts,
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

fn append_text_segment(output: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(text);
}

fn join_part_text<'a>(parts: impl IntoIterator<Item = &'a WorkPart>) -> String {
    let mut output = String::new();
    for part in parts {
        append_text_segment(&mut output, &part.text);
    }
    output
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
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
            index: input_count,
            occurred_at: entry.ended_at.clone(),
            label: None,
            text: prompt.to_string(),
            ansi: entry.prompt_ansi.clone(),
            tags: Vec::new(),
        });
    }
    if !command.is_empty() {
        input_count += 1;
        parts.push(WorkPart {
            io: WorkPartIo::Input,
            kind: WorkPartKind::Command,
            index: input_count,
            occurred_at: entry.ended_at.clone(),
            label: None,
            text: command.to_string(),
            ansi: None,
            tags: Vec::new(),
        });
    }
    if !output.is_empty() {
        output_count += 1;
        parts.push(WorkPart {
            io: WorkPartIo::Output,
            kind: WorkPartKind::Text,
            index: output_count,
            occurred_at: entry.ended_at.clone(),
            label: None,
            text: output.to_string(),
            ansi: entry.output_ansi.clone(),
            tags: Vec::new(),
        });
    }
    parts
}

fn agent_parts(blocks: &[AgentBlock]) -> Vec<WorkPart> {
    let mut parts = Vec::new();
    let mut input_count = 0;
    let mut output_count = 0;
    for block in blocks {
        let text = compact_skill_blocks(block.text.trim());
        if text.is_empty() {
            continue;
        }
        let (io, kind) = match block.kind {
            AgentBlockKind::User => (WorkPartIo::Input, WorkPartKind::UserMessage),
            AgentBlockKind::Assistant => (WorkPartIo::Output, WorkPartKind::AssistantMessage),
            AgentBlockKind::ToolCall => (WorkPartIo::Input, WorkPartKind::ToolCall),
            AgentBlockKind::ToolOutput => (WorkPartIo::Output, WorkPartKind::ToolOutput),
        };
        let index = match io {
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
            index,
            occurred_at: block.timestamp.clone(),
            label: block.label.clone(),
            text,
            ansi: None,
            tags: Vec::new(),
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
        assert_eq!(
            record.status.as_ref().map(|status| status.outcome),
            Some(WorkOutcome::Failure)
        );
        assert_eq!(
            record.status.as_ref().and_then(|status| status.exit_code),
            Some(101)
        );
        assert_eq!(record.input_text().as_deref(), Some("cargo test"));
        assert_eq!(record.output_text().as_deref(), Some("failed"));
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
            records[0].input_text().as_deref(),
            Some("fix latest terminal error")
        );
        assert_eq!(records[0].output_text(), None);
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
        assert_eq!(records[0].input_text().as_deref(), Some("implement this"));
        assert_eq!(records[0].output_text().as_deref(), Some("done"));
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
            records[0].input_text().as_deref(),
            Some("[skill:sivtr-memory]\n\nreal task")
        );
        assert_eq!(records[0].output_text().as_deref(), Some("done"));
        assert_eq!(records[0].parts.len(), 2);
        assert_eq!(records[0].parts[0].kind, WorkPartKind::UserMessage);
        assert_eq!(
            records[0].parts[0].text,
            "[skill:sivtr-memory]\n\nreal task"
        );
        assert_eq!(
            records[0].parts[0]
                .occurred_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:01:00Z")
        );
        assert_eq!(records[0].parts[1].kind, WorkPartKind::AssistantMessage);
        assert_eq!(records[0].parts[1].text, "done");
        assert_eq!(
            records[0].parts[1]
                .occurred_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:02:00Z")
        );
    }
}
