use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;
use std::io::Write;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::command_blocks::CommandBlockSpan;
use crate::command_blocks::ParsedCommandBlock as CommandBlock;
use crate::commands::command_block_selector::{parse_selector, resolve_selector, CommandSelection};
use sivtr_core::capture::scrollback;
use sivtr_core::codex::{
    find_current_session, format_blocks, parse_session_file, CodexBlock, CodexBlockKind,
    CodexSession,
};
use sivtr_core::session::{self, SessionEntry};

mod picker;

use picker::{run_picker, PickEntry};

const PICK_LIMIT: usize = 50;
const PICK_PREVIEW_LINES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMode {
    Both,
    InputOnly,
    OutputOnly,
    CommandOnly,
}

#[derive(Clone, Copy, Debug)]
pub struct CopyRequest<'a> {
    pub selector: Option<&'a str>,
    pub pick: bool,
    pub mode: CopyMode,
    pub include_prompt: bool,
    pub prompt_override: Option<&'a str>,
    pub print_full: bool,
    pub ansi: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexSelectionMode {
    LastTurn,
    LastAssistant,
    LastUser,
    LastTool,
    All,
}

#[derive(Clone, Copy, Debug)]
pub struct CodexCopyRequest<'a> {
    pub selector: Option<&'a str>,
    pub pick: bool,
    pub selection_mode: CodexSelectionMode,
    pub print_full: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

#[derive(Clone, Debug)]
struct IndexedCommandBlock {
    plain: CommandBlock,
    ansi: Option<CommandBlock>,
}

impl IndexedCommandBlock {
    fn from_session_entry(entry: &SessionEntry) -> Self {
        let plain = CommandBlock::from_session_entry(entry);
        let ansi = entry.has_ansi().then(|| CommandBlock {
            input_with_prompt: entry.render_input_ansi(),
            input_without_prompt: plain.input_without_prompt.clone(),
            output: entry
                .output_ansi
                .clone()
                .unwrap_or_else(|| plain.output.clone()),
            command: plain.command.clone(),
        });

        Self { plain, ansi }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TextPair {
    plain: String,
    ansi: String,
}

#[derive(Clone, Debug)]
enum PickerTuiTarget {
    SessionLog,
    Text(VimView),
}

#[derive(Clone, Debug)]
struct VimView {
    raw: String,
    blocks: Vec<VimBlock>,
    alternate: Option<VimAlternateView>,
}

#[derive(Clone, Debug)]
struct VimAlternateView {
    label: String,
    raw: String,
    blocks: Vec<VimBlock>,
}

#[derive(Clone, Debug, Serialize)]
struct VimBlock {
    start: usize,
    end: usize,
    input_start: usize,
    input_end: usize,
    output_start: usize,
    output_end: usize,
    block_text: String,
    input_text: String,
    output_text: String,
    command_text: String,
}

/// Copy recent command blocks to clipboard.
pub fn execute(request: CopyRequest<'_>) -> Result<()> {
    let CopyRequest {
        selector,
        pick,
        mode,
        include_prompt,
        prompt_override,
        print_full,
        ansi,
        regex,
        lines,
    } = request;

    let log_path = scrollback::session_log_path();
    if !log_path.exists() {
        eprintln!("sivtr: no session log found");
        eprintln!("  hint: run `sivtr init <shell>`, restart the shell, then run some commands");
        return Ok(());
    }

    let entries = session::load_entries(&log_path).context("Failed to read session log")?;
    if entries.is_empty() {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a few commands first, then try `sivtr copy` again");
        return Ok(());
    }

    let blocks: Vec<IndexedCommandBlock> = entries
        .iter()
        .map(IndexedCommandBlock::from_session_entry)
        .collect();

    let total = blocks.len();
    if total == 0 {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a command first, then try `sivtr copy` again");
        return Ok(());
    }

    let selection = if pick {
        pick_selection(&blocks)?
    } else {
        parse_selector(selector.unwrap_or("1"))?
    };

    let indices = resolve_selector(selection, total)?;
    if indices.is_empty() {
        eprintln!("sivtr: nothing selected");
        eprintln!("  hint: choose at least one command block");
        return Ok(());
    }

    let copied_blocks: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| blocks.get(*idx))
        .map(|block| format_block_pair(block, mode, include_prompt, prompt_override))
        .filter(|block| !block.plain.trim().is_empty())
        .collect();

    if copied_blocks.is_empty() {
        eprintln!("sivtr: selected commands are empty");
        eprintln!("  hint: try `sivtr copy --out` or choose a different block");
        return Ok(());
    }

    let mut text = join_text_pairs(&copied_blocks, "\n\n");

    if let Some(pattern) = regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    let text = if ansi {
        text.ansi.trim().to_string()
    } else {
        text.plain.trim().to_string()
    };
    finish_copy(
        text,
        print_full,
        format!("sivtr: copied {} command(s) to clipboard", indices.len()),
    )
}

pub fn execute_codex(request: CodexCopyRequest<'_>) -> Result<()> {
    let path = resolve_codex_session_path()?;
    let session = parse_session_file(&path)?;

    if session.blocks.is_empty() {
        eprintln!("sivtr: Codex session has no parsed conversation blocks");
        return Ok(());
    }

    let units = build_codex_units(&session, request.selection_mode);
    if units.is_empty() {
        eprintln!("sivtr: selected Codex content is empty");
        return Ok(());
    }

    let selection = if request.pick {
        pick_text_selection(
            &units,
            "sivtr copy codex --pick",
            build_codex_vim_view(&session.blocks),
        )?
    } else {
        parse_selector(request.selector.unwrap_or("1"))?
    };
    let indices = resolve_selector(selection, units.len())?;
    let selected_units: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| units.get(*idx).cloned())
        .filter(|unit| !unit.plain.trim().is_empty())
        .collect();
    if selected_units.is_empty() {
        eprintln!("sivtr: selected Codex content is empty");
        return Ok(());
    }

    let mut text = join_text_pairs(&selected_units, "\n\n");

    if let Some(pattern) = request.regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = request.lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    finish_copy(
        text.plain.trim().to_string(),
        request.print_full,
        "sivtr: copied Codex content to clipboard".to_string(),
    )
}

fn format_block_pair(
    block: &IndexedCommandBlock,
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> TextPair {
    let plain = format_block(&block.plain, mode, include_prompt, prompt_override);
    let ansi = format_block(
        block.ansi.as_ref().unwrap_or(&block.plain),
        mode,
        include_prompt,
        prompt_override,
    );

    TextPair { plain, ansi }
}

fn join_text_pairs(pairs: &[TextPair], separator: &str) -> TextPair {
    TextPair {
        plain: pairs
            .iter()
            .map(|pair| pair.plain.as_str())
            .collect::<Vec<_>>()
            .join(separator),
        ansi: pairs
            .iter()
            .map(|pair| pair.ansi.as_str())
            .collect::<Vec<_>>()
            .join(separator),
    }
}

fn format_block(
    block: &CommandBlock,
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> String {
    match mode {
        CopyMode::Both => {
            let input = if include_prompt {
                format_input(block, prompt_override)
            } else {
                block.input_without_prompt.clone()
            };
            match (input.is_empty(), block.output.is_empty()) {
                (false, false) => format!("{}\n{}", input, block.output),
                (false, true) => input,
                (true, false) => block.output.clone(),
                (true, true) => String::new(),
            }
        }
        CopyMode::InputOnly => {
            if include_prompt {
                format_input(block, prompt_override)
            } else {
                block.input_without_prompt.clone()
            }
        }
        CopyMode::OutputOnly => block.output.clone(),
        CopyMode::CommandOnly => block.command.clone(),
    }
}

fn format_input(block: &CommandBlock, prompt_override: Option<&str>) -> String {
    match prompt_override {
        Some(prompt) if !block.command.is_empty() => render_prompt_override(prompt, &block.command),
        Some(_) => block.input_with_prompt.clone(),
        None => block.input_with_prompt.clone(),
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

fn pick_selection(blocks: &[IndexedCommandBlock]) -> Result<CommandSelection> {
    let total = blocks.len();
    let shown = total.min(PICK_LIMIT);
    let entries: Vec<PickEntry> = blocks
        .iter()
        .rev()
        .take(shown)
        .enumerate()
        .map(|(offset, block)| {
            let recent = offset + 1;
            let output_preview = build_output_preview(&block.plain);
            let preview = if !block.plain.command.is_empty() {
                block.plain.command.clone()
            } else if !block.plain.output.is_empty() {
                block.plain.output.lines().next().unwrap_or("").to_string()
            } else {
                "<empty>".to_string()
            };
            PickEntry {
                recent,
                preview,
                output_preview,
                full_preview: block.plain.output.clone(),
                selected: false,
            }
        })
        .collect();

    run_picker(
        entries,
        total,
        "sivtr copy --pick",
        PickerTuiTarget::SessionLog,
    )
}

fn filter_lines_by_regex(text: &TextPair, pattern: &str) -> Result<TextPair> {
    let regex = Regex::new(pattern)
        .with_context(|| format!("Invalid regex `{pattern}`. Check the pattern syntax."))?;
    let indices = text
        .plain
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| regex.is_match(line).then_some(idx))
        .collect::<Vec<_>>();
    Ok(select_lines(text, &indices))
}

fn filter_lines_by_spec(text: &TextPair, spec: &str) -> Result<TextPair> {
    let lines: Vec<&str> = text.plain.lines().collect();
    let mut selected = Vec::new();

    for part in spec
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let range = part.split_once(':');

        if let Some((start, end)) = range {
            let start = parse_line_number(start)?;
            let end = parse_line_number(end)?;
            if start == 0 || end == 0 {
                anyhow::bail!("Line ranges are 1-based. Example: `10:20`.");
            }
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            for idx in start..=end {
                if lines.get(idx - 1).is_some() {
                    selected.push(idx - 1);
                }
            }
        } else {
            let idx = parse_line_number(part)?;
            if idx == 0 {
                anyhow::bail!("Line numbers are 1-based. Example: `1,3,8:12`.");
            }
            if lines.get(idx - 1).is_some() {
                selected.push(idx - 1);
            }
        }
    }

    Ok(select_lines(text, &selected))
}

fn select_lines(text: &TextPair, indices: &[usize]) -> TextPair {
    let plain_lines: Vec<&str> = text.plain.lines().collect();
    let ansi_lines: Vec<&str> = text.ansi.lines().collect();
    let mut plain_selected = Vec::new();
    let mut ansi_selected = Vec::new();

    for &idx in indices {
        if let Some(line) = plain_lines.get(idx) {
            plain_selected.push((*line).to_string());
            ansi_selected.push(ansi_lines.get(idx).copied().unwrap_or(line).to_string());
        }
    }

    TextPair {
        plain: plain_selected.join("\n"),
        ansi: ansi_selected.join("\n"),
    }
}

fn parse_line_number(value: &str) -> Result<usize> {
    value.parse::<usize>().with_context(|| {
        format!("Invalid line number `{value}`. Use `N`, `A:B`, or comma-separated lists.")
    })
}

fn finish_copy(text: String, print_full: bool, success_message: String) -> Result<()> {
    if text.is_empty() {
        eprintln!("sivtr: filters removed everything");
        eprintln!("  hint: loosen `--regex` or `--lines`, or copy without filters");
        return Ok(());
    }

    arboard::Clipboard::new()
        .context("Failed to open clipboard")?
        .set_text(&text)
        .context("Failed to set clipboard")?;

    if print_full {
        for line in text.lines() {
            eprintln!("  {line}");
        }
    }

    eprintln!("{success_message}");
    Ok(())
}

fn resolve_codex_session_path() -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    find_current_session(&cwd)?.context("No Codex sessions found")
}

fn build_codex_units(session: &CodexSession, selection_mode: CodexSelectionMode) -> Vec<TextPair> {
    match selection_mode {
        CodexSelectionMode::LastTurn => build_codex_turn_units(session),
        CodexSelectionMode::LastAssistant => {
            build_codex_kind_units(session, CodexBlockKind::Assistant)
        }
        CodexSelectionMode::LastUser => build_codex_kind_units(session, CodexBlockKind::User),
        CodexSelectionMode::LastTool => build_codex_kind_units(session, CodexBlockKind::ToolOutput),
        CodexSelectionMode::All => vec![TextPair {
            plain: format_blocks(&session.blocks),
            ansi: String::new(),
        }],
    }
}

fn build_codex_turn_units(session: &CodexSession) -> Vec<TextPair> {
    let mut turns = Vec::new();

    for (idx, block) in session.blocks.iter().enumerate() {
        if block.kind != CodexBlockKind::Assistant {
            continue;
        }

        let start = session.blocks[..idx]
            .iter()
            .rposition(|block| block.kind == CodexBlockKind::User)
            .unwrap_or(idx);

        let turn_blocks: Vec<CodexBlock> = session.blocks[start..=idx]
            .iter()
            .filter(|block| matches!(block.kind, CodexBlockKind::User | CodexBlockKind::Assistant))
            .cloned()
            .collect();

        let text = format_blocks(&turn_blocks);
        if !text.trim().is_empty() {
            turns.push(TextPair {
                plain: text,
                ansi: String::new(),
            });
        }
    }

    turns
}

fn build_codex_kind_units(session: &CodexSession, kind: CodexBlockKind) -> Vec<TextPair> {
    session
        .blocks
        .iter()
        .filter(|block| block.kind == kind)
        .map(|block| TextPair {
            plain: block.text.clone(),
            ansi: String::new(),
        })
        .collect()
}

fn build_codex_vim_view(blocks: &[CodexBlock]) -> VimView {
    let mut main_turns = Vec::new();
    let mut full_turns = Vec::new();

    for (idx, block) in blocks.iter().enumerate() {
        if block.kind != CodexBlockKind::Assistant {
            continue;
        }

        let start = blocks[..idx]
            .iter()
            .rposition(|block| block.kind == CodexBlockKind::User)
            .unwrap_or(idx);
        let full_blocks = &blocks[start..=idx];
        let main_blocks: Vec<CodexBlock> = full_blocks
            .iter()
            .filter(|block| matches!(block.kind, CodexBlockKind::User | CodexBlockKind::Assistant))
            .cloned()
            .collect();

        if !main_blocks.is_empty() {
            main_turns.push(main_blocks);
            full_turns.push(full_blocks.to_vec());
        }
    }

    if main_turns.is_empty() {
        main_turns = blocks
            .iter()
            .filter(|block| matches!(block.kind, CodexBlockKind::User | CodexBlockKind::Assistant))
            .cloned()
            .map(|block| vec![block])
            .collect();
        full_turns = main_turns.clone();
    }

    let main = build_codex_turn_view(&main_turns);
    let full = build_codex_turn_view(&full_turns);

    VimView {
        raw: main.raw,
        blocks: main.blocks,
        alternate: Some(VimAlternateView {
            label: "tools".to_string(),
            raw: full.raw,
            blocks: full.blocks,
        }),
    }
}

fn build_codex_turn_view(turns: &[Vec<CodexBlock>]) -> VimAlternateView {
    let mut rendered_turns = Vec::new();
    let mut raw_parts = Vec::new();
    let mut next_line = 1usize;

    for turn in turns {
        let rendered_blocks: Vec<String> = turn.iter().map(format_codex_block_for_vim).collect();
        let rendered = rendered_blocks.join("\n\n");
        if rendered.trim().is_empty() {
            continue;
        }

        let line_count = line_count(&rendered);
        let start = next_line;
        let end = start + line_count.saturating_sub(1);
        let input_text = join_codex_kind_text(turn, CodexBlockKind::User);
        let output_text = turn
            .iter()
            .filter(|block| {
                matches!(
                    block.kind,
                    CodexBlockKind::Assistant | CodexBlockKind::ToolOutput
                )
            })
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let command_text = turn
            .iter()
            .filter(|block| block.kind == CodexBlockKind::ToolCall)
            .filter_map(|block| block.label.as_deref())
            .collect::<Vec<_>>()
            .join("\n");

        rendered_turns.push(VimBlock {
            start,
            end,
            input_start: range_for_first_kind(turn, CodexBlockKind::User, start).0,
            input_end: range_for_first_kind(turn, CodexBlockKind::User, start).1,
            output_start: range_for_first_kind(turn, CodexBlockKind::Assistant, start).0,
            output_end: range_for_first_kind(turn, CodexBlockKind::Assistant, start).1,
            block_text: rendered.clone(),
            input_text,
            output_text,
            command_text,
        });
        raw_parts.push(rendered);
        next_line = end + 2;
    }

    VimAlternateView {
        label: "main".to_string(),
        raw: raw_parts.join("\n\n"),
        blocks: rendered_turns,
    }
}

fn join_codex_kind_text(turn: &[CodexBlock], kind: CodexBlockKind) -> String {
    turn.iter()
        .filter(|block| block.kind == kind)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn range_for_first_kind(
    turn: &[CodexBlock],
    kind: CodexBlockKind,
    turn_start_line: usize,
) -> (usize, usize) {
    let mut cursor = turn_start_line;
    for block in turn {
        let rendered = format_codex_block_for_vim(block);
        let count = line_count(&rendered);
        if block.kind == kind {
            let body_start = (cursor + 1).min(cursor + count.saturating_sub(1));
            return (body_start, cursor + count.saturating_sub(1));
        }
        cursor += count + 2;
    }
    (0, 0)
}

fn format_codex_block_for_vim(block: &CodexBlock) -> String {
    let heading = match block.kind {
        CodexBlockKind::User => "User".to_string(),
        CodexBlockKind::Assistant => "Assistant".to_string(),
        CodexBlockKind::ToolCall => block
            .label
            .as_deref()
            .map(|label| format!("Tool Call: {label}"))
            .unwrap_or_else(|| "Tool Call".to_string()),
        CodexBlockKind::ToolOutput => "Tool Output".to_string(),
    };
    format!("## {heading}\n{}", block.text.trim())
}

fn build_session_vim_view(raw: String) -> Result<VimView> {
    let spans = crate::command_blocks::load_from_session_log()?.unwrap_or_default();
    Ok(VimView {
        raw,
        blocks: spans.iter().map(VimBlock::from_command_span).collect(),
        alternate: None,
    })
}

impl VimBlock {
    fn from_command_span(span: &CommandBlockSpan) -> Self {
        let (input_start, input_end) = one_based_range(span.input_line_range);
        let (output_start, output_end) = one_based_range(span.output_line_range);
        Self {
            start: span.line_start + 1,
            end: span.line_end + 1,
            input_start,
            input_end,
            output_start,
            output_end,
            block_text: span
                .text_for(crate::command_blocks::CopyTarget::Block)
                .unwrap_or_default(),
            input_text: span
                .text_for(crate::command_blocks::CopyTarget::Input)
                .unwrap_or_default(),
            output_text: span
                .text_for(crate::command_blocks::CopyTarget::Output)
                .unwrap_or_default(),
            command_text: span
                .text_for(crate::command_blocks::CopyTarget::Command)
                .unwrap_or_default(),
        }
    }
}

fn one_based_range(range: Option<(usize, usize)>) -> (usize, usize) {
    range
        .map(|(start, end)| (start + 1, end + 1))
        .unwrap_or((0, 0))
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn pick_text_selection(
    units: &[TextPair],
    title: &str,
    vim_view: VimView,
) -> Result<CommandSelection> {
    let total = units.len();
    let shown = total.min(PICK_LIMIT);
    let entries: Vec<PickEntry> = units
        .iter()
        .rev()
        .take(shown)
        .enumerate()
        .map(|(offset, unit)| PickEntry {
            recent: offset + 1,
            preview: build_text_preview(&unit.plain),
            output_preview: build_text_preview_lines(&unit.plain),
            full_preview: unit.plain.clone(),
            selected: false,
        })
        .collect();

    run_picker(entries, total, title, PickerTuiTarget::Text(vim_view))
}

#[cfg(test)]
mod tests {
    use super::picker::{apply_range_toggle, selection_from_entries, PickEntry};
    use super::{
        build_codex_vim_view, build_output_preview, filter_lines_by_regex, filter_lines_by_spec,
        format_block, is_vim_command, vim_single_quote, CodexBlock, CodexBlockKind, CommandBlock,
        CommandSelection, CopyMode, TextPair,
    };

    #[test]
    fn formats_modes() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> git status --all -a".to_string(),
            input_without_prompt: "git status --all -a".to_string(),
            output: "clean".to_string(),
            command: "git status --all -a".to_string(),
        };
        assert_eq!(
            format_block(&block, CopyMode::Both, false, None),
            "git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true, None),
            "PS C:\\repo> git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, false, None),
            "git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true, None),
            "PS C:\\repo> git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::OutputOnly, false, None),
            "clean"
        );
        assert_eq!(
            format_block(&block, CopyMode::CommandOnly, false, None),
            "git status --all -a"
        );
    }

    #[test]
    fn rewrites_prompt_in_copied_input() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> cargo test".to_string(),
            input_without_prompt: "cargo test".to_string(),
            output: "ok".to_string(),
            command: "cargo test".to_string(),
        };

        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true, Some(":")),
            ": cargo test"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true, Some(">>>")),
            ">>> cargo test\nok"
        );
    }

    #[test]
    fn picker_selection_keeps_disjoint_blocks() {
        let selection = selection_from_entries(&[
            PickEntry {
                recent: 1,
                preview: "latest".to_string(),
                output_preview: "out1".to_string(),
                full_preview: "out1".to_string(),
                selected: true,
            },
            PickEntry {
                recent: 2,
                preview: "second".to_string(),
                output_preview: "out2".to_string(),
                full_preview: "out2".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 4,
                preview: "fourth".to_string(),
                output_preview: "out4".to_string(),
                full_preview: "out4".to_string(),
                selected: true,
            },
        ])
        .unwrap();

        assert_eq!(selection, CommandSelection::RecentExplicit(vec![1, 4]));
    }

    #[test]
    fn filters_by_regex() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\nwarn: b\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.plain, "warn: b");
    }

    #[test]
    fn filters_ansi_by_plain_regex_matches() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\n\x1b[31mwarn: b\x1b[0m\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.ansi, "\x1b[31mwarn: b\x1b[0m");
    }

    #[test]
    fn filters_by_line_spec_with_colon_ranges() {
        let filtered = filter_lines_by_spec(
            &TextPair {
                plain: "a\nb\nc\nd".to_string(),
                ansi: "a\nb\nc\nd".to_string(),
            },
            "2,4:3",
        )
        .unwrap();
        assert_eq!(filtered.plain, "b\nc\nd");
    }

    #[test]
    fn rejects_dash_ranges_for_lines() {
        assert!(filter_lines_by_spec(
            &TextPair {
                plain: "a\nb\nc".to_string(),
                ansi: "a\nb\nc".to_string(),
            },
            "1-2"
        )
        .is_err());
    }

    #[test]
    fn toggles_selected_range_in_picker() {
        let mut entries = vec![
            PickEntry {
                recent: 1,
                preview: "one".to_string(),
                output_preview: "out1".to_string(),
                full_preview: "out1".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 2,
                preview: "two".to_string(),
                output_preview: "out2".to_string(),
                full_preview: "out2".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 3,
                preview: "three".to_string(),
                output_preview: "out3".to_string(),
                full_preview: "out3".to_string(),
                selected: true,
            },
        ];

        apply_range_toggle(&mut entries, 0, 2);
        assert!(entries.iter().all(|entry| entry.selected));

        apply_range_toggle(&mut entries, 0, 2);
        assert!(entries.iter().all(|entry| !entry.selected));
    }

    #[test]
    fn builds_output_preview_from_first_lines() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> cargo test".to_string(),
            input_without_prompt: "cargo test".to_string(),
            output: "line1\nline2\nline3".to_string(),
            command: "cargo test".to_string(),
        };

        assert_eq!(build_output_preview(&block), "line1\nline2\nline3");
    }

    #[test]
    fn detects_vim_compatible_editor_commands() {
        assert!(is_vim_command("nvim"));
        assert!(is_vim_command("vim -Nu NONE"));
        assert!(is_vim_command("C:\\Tools\\gVim\\gvim.exe"));
        assert!(!is_vim_command("code --wait"));
        assert!(!is_vim_command("hx"));
    }

    #[test]
    fn escapes_vim_single_quoted_strings() {
        assert_eq!(
            vim_single_quote("C:\\tmp\\it's.json"),
            "C:\\tmp\\it''s.json"
        );
    }

    #[test]
    fn codex_vim_view_hides_tools_by_default_and_moves_by_turn() {
        let blocks = vec![
            CodexBlock {
                kind: CodexBlockKind::User,
                timestamp: None,
                label: None,
                text: "first question".to_string(),
            },
            CodexBlock {
                kind: CodexBlockKind::ToolCall,
                timestamp: None,
                label: Some("shell".to_string()),
                text: "tool call".to_string(),
            },
            CodexBlock {
                kind: CodexBlockKind::ToolOutput,
                timestamp: None,
                label: None,
                text: "tool output".to_string(),
            },
            CodexBlock {
                kind: CodexBlockKind::Assistant,
                timestamp: None,
                label: None,
                text: "first answer".to_string(),
            },
            CodexBlock {
                kind: CodexBlockKind::User,
                timestamp: None,
                label: None,
                text: "second question".to_string(),
            },
            CodexBlock {
                kind: CodexBlockKind::Assistant,
                timestamp: None,
                label: None,
                text: "second answer".to_string(),
            },
        ];

        let view = build_codex_vim_view(&blocks);

        assert!(!view.raw.contains("tool output"));
        assert_eq!(view.blocks.len(), 2);
        assert_eq!(view.blocks[0].start, 1);
        assert_eq!(view.blocks[1].start, view.blocks[0].end + 2);
        assert_eq!(view.blocks[0].input_text, "first question");
        assert_eq!(view.blocks[0].output_text, "first answer");

        let full = view.alternate.expect("tools view should exist");
        assert!(full.raw.contains("tool output"));
        assert_eq!(full.blocks.len(), 2);
        assert!(full.blocks[0].output_text.contains("tool output"));
        assert!(full.blocks[0].output_text.contains("first answer"));
    }
}

fn open_picker_vim(target: &PickerTuiTarget) -> Result<()> {
    let view = match target {
        PickerTuiTarget::SessionLog => match scrollback::read_session_log()? {
            Some(raw) if !raw.trim().is_empty() => build_session_vim_view(raw)?,
            _ => {
                eprintln!("sivtr: session log is empty");
                return Ok(());
            }
        },
        PickerTuiTarget::Text(view) => view.clone(),
    };
    open_vim_view(&view)
}

fn open_vim_view(view: &VimView) -> Result<()> {
    let editor = resolve_vim_editor()?;
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let content_path = dir.join(format!("sivtr-view-{}-{nonce}.txt", std::process::id()));
    let vimrc_path = dir.join(format!("sivtr-view-{}-{nonce}.vim", std::process::id()));
    let blocks_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.blocks.json",
        std::process::id()
    ));
    let alternate_content_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.tools.txt",
        std::process::id()
    ));
    let alternate_blocks_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.tools.blocks.json",
        std::process::id()
    ));

    std::fs::write(&content_path, &view.raw).context("Failed to write Vim view file")?;
    let blocks_json =
        serde_json::to_string(&view.blocks).context("Failed to encode Vim block data")?;
    std::fs::write(&blocks_path, blocks_json).context("Failed to write Vim block data")?;
    let alternate = if let Some(alternate) = &view.alternate {
        std::fs::write(&alternate_content_path, &alternate.raw)
            .context("Failed to write alternate Vim view file")?;
        let blocks_json = serde_json::to_string(&alternate.blocks)
            .context("Failed to encode alternate Vim block data")?;
        std::fs::write(&alternate_blocks_path, blocks_json)
            .context("Failed to write alternate Vim block data")?;
        Some((
            alternate.label.as_str(),
            alternate_content_path.as_path(),
            alternate_blocks_path.as_path(),
        ))
    } else {
        None
    };
    write_vimrc(&vimrc_path, &blocks_path, alternate)?;

    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (program, extra_args) = parts
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Empty Vim editor command"))?;

    let status = Command::new(program)
        .args(extra_args)
        .arg("-u")
        .arg(&vimrc_path)
        .arg("-n")
        .arg("-R")
        .arg(&content_path)
        .status()
        .with_context(|| format!("Failed to launch Vim editor `{editor}`"))?;

    let _ = std::fs::remove_file(&content_path);
    let _ = std::fs::remove_file(&vimrc_path);
    let _ = std::fs::remove_file(&blocks_path);
    let _ = std::fs::remove_file(&alternate_content_path);
    let _ = std::fs::remove_file(&alternate_blocks_path);

    if !status.success() {
        anyhow::bail!("Vim editor `{editor}` exited with {status}");
    }
    Ok(())
}

fn resolve_vim_editor() -> Result<String> {
    let config = sivtr_core::config::SivtrConfig::load().unwrap_or_default();
    if is_vim_command(&config.editor.command) {
        return Ok(config.editor.command);
    }

    for candidate in ["nvim", "vim", "vi"] {
        if command_exists(candidate) {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("No Vim-compatible editor found. Set `editor.command` to nvim/vim/vi.")
}

fn is_vim_command(command: &str) -> bool {
    let Some(program) = command.split_whitespace().next() else {
        return false;
    };
    let name = std::path::Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_lowercase();
    name == "vi" || name.contains("vim")
}

fn vim_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn command_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("which")
            .arg(name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

fn write_vimrc(
    path: &std::path::Path,
    blocks_path: &std::path::Path,
    alternate: Option<(&str, &std::path::Path, &std::path::Path)>,
) -> Result<()> {
    let mut file = std::fs::File::create(path).context("Failed to create temporary Vim config")?;
    let blocks_path = vim_single_quote(&blocks_path.to_string_lossy());
    let (alternate_label, alternate_content_path, alternate_blocks_path) =
        if let Some((label, content, blocks)) = alternate {
            (
                vim_single_quote(label),
                vim_single_quote(&content.to_string_lossy()),
                vim_single_quote(&blocks.to_string_lossy()),
            )
        } else {
            (String::new(), String::new(), String::new())
        };
    let script = format!(
        r#"
set nocompatible
set nomodeline
set readonly
set nomodifiable
set nomodified
set number
set nowrap
set nofoldenable
let s:sivtr_blocks = json_decode(join(readfile('{blocks_path}'), "\n"))
let s:sivtr_main_blocks_path = '{blocks_path}'
let s:sivtr_alt_label = '{alternate_label}'
let s:sivtr_alt_content_path = '{alternate_content_path}'
let s:sivtr_alt_blocks_path = '{alternate_blocks_path}'
let s:sivtr_tools_visible = 0

function! s:SivtrLoadBlocks(path) abort
  let s:sivtr_blocks = json_decode(join(readfile(a:path), "\n"))
endfunction

function! s:SivtrToggleTools() abort
  if empty(s:sivtr_alt_content_path)
    echo 'sivtr: no alternate view'
    return
  endif
  let l:top = winsaveview()
  setlocal modifiable
  silent %delete _
  if s:sivtr_tools_visible
    silent execute '0read ' . fnameescape(expand('%:p'))
    silent 1delete _
    call s:SivtrLoadBlocks(s:sivtr_main_blocks_path)
    let s:sivtr_tools_visible = 0
    echo 'sivtr: tools hidden'
  else
    silent execute '0read ' . fnameescape(s:sivtr_alt_content_path)
    silent 1delete _
    call s:SivtrLoadBlocks(s:sivtr_alt_blocks_path)
    let s:sivtr_tools_visible = 1
    echo 'sivtr: ' . s:sivtr_alt_label . ' visible'
  endif
  setlocal nomodifiable nomodified readonly
  call winrestview(l:top)
endfunction

function! s:SivtrCurrentBlockIndex() abort
  let l:line = line('.')
  let l:fallback = -1
  for l:i in range(0, len(s:sivtr_blocks) - 1)
    let l:block = s:sivtr_blocks[l:i]
    if l:line >= l:block.start && l:line <= l:block.end
      return l:i
    endif
    if l:block.start <= l:line
      let l:fallback = l:i
    endif
  endfor
  return l:fallback >= 0 ? l:fallback : 0
endfunction

function! s:SivtrCurrentBlock() abort
  if empty(s:sivtr_blocks)
    echohl ErrorMsg | echo 'sivtr: no blocks' | echohl None
    return {{}}
  endif
  return s:sivtr_blocks[s:SivtrCurrentBlockIndex()]
endfunction

function! s:SivtrJump(delta) abort
  if empty(s:sivtr_blocks)
    echohl ErrorMsg | echo 'sivtr: no blocks' | echohl None
    return
  endif
  let l:idx = s:SivtrCurrentBlockIndex() + a:delta
  let l:idx = max([0, min([l:idx, len(s:sivtr_blocks) - 1])])
  call cursor(s:sivtr_blocks[l:idx].start, 1)
  normal! zz
endfunction

function! s:SivtrCopy(kind) abort
  let l:block = s:SivtrCurrentBlock()
  if empty(l:block)
    return
  endif
  let l:key = a:kind . '_text'
  let l:text = get(l:block, l:key, '')
  if empty(l:text)
    echohl ErrorMsg | echo 'sivtr: current block has no ' . a:kind . ' content' | echohl None
    return
  endif
  call setreg('"', l:text)
  try | call setreg('+', l:text) | catch | endtry
  try | call setreg('*', l:text) | catch | endtry
  echo 'sivtr: copied current ' . a:kind
endfunction

function! s:SivtrSelect(kind) abort
  let l:block = s:SivtrCurrentBlock()
  if empty(l:block)
    return
  endif
  if a:kind ==# 'block'
    let [l:start, l:end] = [l:block.start, l:block.end]
  elseif a:kind ==# 'input'
    let [l:start, l:end] = [l:block.input_start, l:block.input_end]
  else
    let [l:start, l:end] = [l:block.output_start, l:block.output_end]
  endif
  if l:start <= 0 || l:end <= 0
    echohl ErrorMsg | echo 'sivtr: current block has no ' . a:kind . ' range' | echohl None
    return
  endif
  call cursor(l:start, 1)
  normal! V
  call cursor(l:end, 1)
endfunction

nnoremap <silent> p :qa!<CR>
nnoremap <silent> q :qa!<CR>
nnoremap <silent> <Esc> :qa!<CR>
nnoremap <silent> [[ :call <SID>SivtrJump(-1)<CR>
nnoremap <silent> ]] :call <SID>SivtrJump(1)<CR>
nnoremap <silent> myy :call <SID>SivtrCopy('block')<CR>
nnoremap <silent> myi :call <SID>SivtrCopy('input')<CR>
nnoremap <silent> myo :call <SID>SivtrCopy('output')<CR>
nnoremap <silent> myc :call <SID>SivtrCopy('command')<CR>
nnoremap <silent> mvv :call <SID>SivtrSelect('block')<CR>
nnoremap <silent> mvi :call <SID>SivtrSelect('input')<CR>
nnoremap <silent> mvo :call <SID>SivtrSelect('output')<CR>
nnoremap <silent> T :call <SID>SivtrToggleTools()<CR>
autocmd VimEnter * echo "sivtr: [[/]] jump turns, T toggles tools, myy/myi/myo/myc copy, mvv/mvi/mvo select, p returns to picker"
"#
    );
    file.write_all(script.as_bytes())
        .context("Failed to write temporary Vim config")?;
    Ok(())
}

fn build_output_preview(block: &CommandBlock) -> String {
    if block.output.trim().is_empty() {
        return "<no output>".to_string();
    }

    let mut lines: Vec<&str> = block.output.lines().take(PICK_PREVIEW_LINES).collect();
    let total_lines = block.output.lines().count();
    if total_lines > PICK_PREVIEW_LINES {
        lines.push("...");
    }
    lines.join("\n")
}

fn build_text_preview(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## "))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

fn build_text_preview_lines(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(PICK_PREVIEW_LINES)
        .collect();
    if text.lines().count() > PICK_PREVIEW_LINES {
        lines.push("...");
    }
    lines.join("\n")
}
