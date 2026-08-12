//! Workspace unit tests.

use super::help::{help_action_for_key, parse_help_key, WorkspaceHelpAction};
use super::layout::can_open_dialogue_vim;
use super::model::{
    WorkspaceCopyParts, WorkspaceDialogue, WorkspaceFocus, WorkspaceSearchView, WorkspaceSource,
};
use super::render::{
    content_title, current_content_dialogue, current_content_ref, line_filter_prompt_text,
    search_box_body, search_box_title,
};
use crate::tui::content::text::{workspace_content_io_texts, workspace_content_text};
use crate::tui::content::view::ContentViewMode;
use crate::tui::search::WorkspaceSearchScope;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

fn tool_test_value(text: String) -> serde_json::Value {
    serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
}

fn part(seq: usize, data: sivtr_core::record::WorkPartData) -> sivtr_core::record::WorkPart {
    sivtr_core::record::WorkPart {
        seq,
        occurred_at: None,
        data,
    }
}

fn chat_record(parts: Vec<sivtr_core::record::WorkPart>) -> WorkRecord {
    WorkRecord {
        schema_version: 2,
        work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
        kind: sivtr_core::record::WorkRecordKind::ChatTurn,
        source: sivtr_core::record::WorkSource {
            channel: sivtr_core::record::WorkChannel::Chat,
            provider: Some("codex".to_string()),
        },
        session: sivtr_core::record::WorkSessionRef {
            id: "session".to_string(),
            canonical_id: None,
            path: None,
        },
        cwd: None,
        time: sivtr_core::record::WorkTime::default(),
        status: None,
        title: "cmd".to_string(),
        parts,
    }
}

fn codex_dialogue(record: WorkRecord) -> WorkspaceDialogue {
    WorkspaceDialogue {
        source: WorkspaceSource::agent(AgentProvider::Codex),
        work_ref: Some(record.work_ref.clone()),
        record: Some(record),
        copy: WorkspaceCopyParts::default(),
    }
}

#[test]
fn can_open_dialogue_vim_accepts_sessions_when_dialogues_exist() {
    assert!(can_open_dialogue_vim(WorkspaceFocus::Sessions, 1));
    assert!(can_open_dialogue_vim(WorkspaceFocus::Dialogues, 1));
    assert!(can_open_dialogue_vim(WorkspaceFocus::Content, 1));
    assert!(!can_open_dialogue_vim(WorkspaceFocus::Sessions, 0));
}

#[test]
fn content_preview_text_preserves_raw_text_without_line_number_prefixes() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "alpha".to_string(),
            },
        ),
        part(
            1,
            sivtr_core::record::WorkPartData::Assistant {
                content: "omega".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Raw,
        None,
    );
    let text = workspace_content_text(&[dialogue], &[false], 0, ContentViewMode::Raw, None);
    assert_eq!(io.input.trim(), "alpha");
    assert_eq!(io.output.trim(), "omega");
    assert!(text.contains("alpha"));
    assert!(text.contains("omega"));
    assert!(!text.contains("## Input"));
    assert!(!text.contains("[r expand]"));
}

#[test]
fn content_preview_text_uses_targeted_part_text_in_raw_mode() {
    let record = chat_record(vec![part(
        1,
        sivtr_core::record::WorkPartData::ToolCall {
            call_id: None,
            tool: Some("tool".to_string()),
            input: tool_test_value("hidden tool call".to_string()),
        },
    )]);
    let dialogue = codex_dialogue(record);

    let text = workspace_content_text(
        &[dialogue],
        &[false],
        0,
        ContentViewMode::Raw,
        Some(WorkAt::Part(1)),
    );
    assert!(text.contains("<:tool:tool call:>"));
    assert!(text.contains("hidden tool call"));
    assert!(text.contains("<:/tool:tool call:>"));
}

#[test]
fn content_preview_text_uses_structured_targeted_part_text_in_reading_mode() {
    let record = chat_record(vec![part(
        1,
        sivtr_core::record::WorkPartData::ToolCall {
            call_id: None,
            tool: Some("tool".to_string()),
            input: tool_test_value("hidden tool call".to_string()),
        },
    )]);
    let dialogue = codex_dialogue(record);

    let text = workspace_content_text(
        &[dialogue],
        &[false],
        0,
        ContentViewMode::Reading,
        Some(WorkAt::Part(1)),
    );

    // Reading folds structure to one open marker only.
    assert_eq!(text.trim(), "<:tool:tool call:>");
    assert!(!text.contains("hidden tool call"));
    assert!(!text.contains("codex/session"));
    assert!(!text.contains("[r expand]"));
}

#[test]
fn reading_mode_folds_structure_and_raw_expands() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "question".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("cargo test".to_string()),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolResult {
                call_id: None,
                tool: Some("Bash".to_string()),
                output: tool_test_value("ok".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::Assistant {
                content: "answer".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading = workspace_content_text(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Reading,
        None,
    );
    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Reading,
        None,
    );
    assert!(reading_io.input.contains("question"));
    // Fold keeps the structural marker so the content renderer can color it.
    assert!(reading_io.output.contains("<:tool:Bash call:>"));
    assert!(!reading_io.output.contains("result"));
    assert!(reading_io.output.contains("answer"));
    assert!(!reading.contains("cargo test"));
    assert!(!reading.contains("codex/session"));
    assert!(!reading.contains("## User"));
    assert!(!reading.contains("## Input"));
    assert!(!reading.contains("[r expand]"));

    let raw_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Raw,
        None,
    );
    let raw = workspace_content_text(&[dialogue], &[false], 0, ContentViewMode::Raw, None);
    assert!(raw_io.input.contains("question"));
    assert!(raw_io.output.contains("cargo test"));
    assert!(raw_io.output.contains("<:tool:Bash call:>"));
    assert!(raw_io.output.contains("<:/tool:Bash call:>"));
    assert!(raw_io.output.contains("<:tool:Bash result:>"));
    assert!(raw_io.output.contains("ok"));
    assert!(raw_io.output.contains("answer"));
    assert!(!raw.contains("codex/session"));
    assert!(!raw.contains("## User"));
    assert!(!raw.contains("## Input"));
}

#[test]
fn reading_mode_collapses_adjacent_structure_runs() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "do it".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("ls".to_string()),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Read".to_string()),
                input: tool_test_value("file".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::Skill {
                skill: Some("review".to_string()),
                content: "skill body".to_string(),
            },
        ),
        part(
            5,
            sivtr_core::record::WorkPartData::Skill {
                skill: Some("deploy".to_string()),
                content: "skill body 2".to_string(),
            },
        ),
        part(
            1,
            sivtr_core::record::WorkPartData::Assistant {
                content: "done".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Reading,
        None,
    );
    let reading = workspace_content_text(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Reading,
        None,
    );
    assert!(reading_io.input.contains("do it"));
    // Tool calls and skills fold to marker-only lines in their respective halves.
    let tool_fold = reading_io
        .output
        .lines()
        .find(|line| line.starts_with("<:tool:"))
        .expect("tools channel line");
    assert!(tool_fold.contains("<:tool:Bash call:>"));
    assert!(tool_fold.contains("<:tool:Read call:>"));
    let skill_fold = reading_io
        .input
        .lines()
        .find(|line| line.starts_with("<:skill:"))
        .expect("skills channel line");
    assert!(skill_fold.contains("<:skill:review:>"));
    assert!(skill_fold.contains("<:skill:deploy:>"));
    assert!(reading_io.output.contains("done"));
    assert!(!reading.contains("file"));
    assert!(!reading.contains("skill body"));
    assert!(!reading.contains("## Input"));
}

#[test]
fn reading_mode_counts_identical_structure_markers_regardless_of_order() {
    let record = chat_record(vec![
        // Interleaved with dialogue — still one IO-section fold, same markers count.
        part(
            1,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("ls".to_string()),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::User {
                content: "middle note".to_string(),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Read".to_string()),
                input: tool_test_value("file".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("pwd".to_string()),
            },
        ),
        part(
            5,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("date".to_string()),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading = workspace_content_text(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Reading,
        None,
    );
    // Repeated tool names count as xN within the single marker line.
    assert!(reading.contains("<:tool:Bash call:> x3 <:tool:Read call:>"));
    assert!(reading.contains("middle note"));
    assert!(!reading.contains("file"));
    assert!(!reading.contains("pwd"));
    // Single tools line for the section (not split by the dialogue part).
    let fold_hits = reading
        .lines()
        .filter(|line| line.starts_with("<:tool:"))
        .count();
    assert_eq!(fold_hits, 1);
}

#[test]
fn reading_mode_keeps_structure_runs_in_call_order() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "do it".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::Assistant {
                content: "checking first".to_string(),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("ls".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::ToolResult {
                call_id: None,
                tool: Some("Bash".to_string()),
                output: tool_test_value("ok".to_string()),
            },
        ),
        part(
            5,
            sivtr_core::record::WorkPartData::Assistant {
                content: "all done".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        &[false],
        0,
        ContentViewMode::Reading,
        None,
    );
    // Fold sits between the assistant chunks, matching the call order.
    let output = &reading_io.output;
    let first_text = output.find("checking first").expect("first assistant text");
    let fold = output.find("<:tool:Bash call:>").expect("tool fold line");
    let last_text = output.find("all done").expect("last assistant text");
    assert!(first_text < fold);
    assert!(fold < last_text);
    assert!(!output.contains("result"));
}

#[test]
fn content_title_includes_view_mode() {
    assert_eq!(
        content_title(ContentViewMode::Reading, &[false, false], None),
        "Content (read/fold)"
    );
    assert_eq!(
        content_title(ContentViewMode::Raw, &[true, false], None),
        "Content (raw/full): 1 dialogue selected"
    );
}

#[test]
fn content_title_includes_current_dialogue_ref() {
    let work_ref = WorkRef::agent(AgentProvider::Codex, "session", 2);

    assert_eq!(
        content_title(ContentViewMode::Reading, &[false], Some(&work_ref)),
        "Content (read/fold) [codex/session/2]"
    );
}

#[test]
fn line_filter_prompt_text_shows_current_input() {
    let prompt = line_filter_prompt_text(Some("2:8"), None, true);
    assert!(prompt.contains("2:8"));
    assert!(prompt.contains("Enter keeps displayed lines."));
}

#[test]
fn line_filter_prompt_text_shows_error_and_current_value() {
    let prompt = line_filter_prompt_text(Some("23"), Some("Invalid line number"), false);
    assert!(prompt.contains("Invalid line number"));
    assert!(prompt.contains("Current: 23"));
}

#[test]
fn parse_help_key_recognizes_named_and_ctrl_specs() {
    use crossterm::event::{KeyCode, KeyModifiers};
    assert_eq!(
        parse_help_key("Tab"),
        Some((KeyCode::Tab, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_help_key("Ctrl-d"),
        Some((KeyCode::Char('d'), KeyModifiers::CONTROL))
    );
    assert_eq!(
        parse_help_key("Space"),
        Some((KeyCode::Char(' '), KeyModifiers::NONE))
    );
    assert_eq!(
        parse_help_key("PgDn"),
        Some((KeyCode::PageDown, KeyModifiers::NONE))
    );
}

#[test]
fn help_action_for_key_is_focus_scoped() {
    use crossterm::event::{KeyCode, KeyModifiers};
    assert_eq!(
        help_action_for_key(KeyCode::Tab, KeyModifiers::NONE, WorkspaceFocus::Content),
        Some(WorkspaceHelpAction::ToggleContentIo)
    );
    assert_eq!(
        help_action_for_key(KeyCode::Tab, KeyModifiers::NONE, WorkspaceFocus::Source),
        None
    );
    // Source-only binding does not fire on Content.
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            WorkspaceFocus::Source
        ),
        Some(WorkspaceHelpAction::SelectAgentSources)
    );
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            WorkspaceFocus::Content
        ),
        Some(WorkspaceHelpAction::ScrollContentTop)
    );
    // Ctrl-d is scroll, bare d is not.
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
            WorkspaceFocus::Content
        ),
        Some(WorkspaceHelpAction::ScrollDown)
    );
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
            WorkspaceFocus::Content
        ),
        None
    );
}

#[test]
fn current_content_dialogue_uses_single_selected_dialogue() {
    let dialogues = vec![
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            record: None,
            copy: WorkspaceCopyParts::default(),
        },
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 2)),
            record: None,
            copy: WorkspaceCopyParts::default(),
        },
    ];

    let current = current_content_dialogue(&dialogues, &[false, true], 0).unwrap();

    assert_eq!(
        current.work_ref.as_ref().unwrap().to_string(),
        "codex/session/2"
    );
}

#[test]
fn current_content_ref_round_trips_active_part_target() {
    let dialogues = vec![WorkspaceDialogue {
        source: WorkspaceSource::agent(AgentProvider::Codex),
        work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 2)),
        record: None,
        copy: WorkspaceCopyParts::default(),
    }];

    let current = current_content_ref(&dialogues, &[false], 0, Some(WorkAt::Part(1))).unwrap();

    assert_eq!(current.to_string(), "codex/session/2/p1");
}

#[test]
fn search_box_body_includes_current_target_ref() {
    let search = WorkspaceSearchView {
        query: "needle",
        scope: WorkspaceSearchScope::Content,
        result_count: 1,
        current_match: Some(0),
        match_count: 1,
        current_target: Some("codex/session/1/4".to_string()),
        input_open: true,
    };

    assert_eq!(search_box_title(&search), "Search  ([1/1])");
    assert_eq!(
        search_box_body(&search),
        "needle\n\nTarget: codex/session/1/4"
    );
}
