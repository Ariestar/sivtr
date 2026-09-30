use super::*;
use crate::agents::{select_blocks, AgentSelection};
use serde_json::json;

const FIXTURE: &str = include_str!("../../../tests/fixtures/dsh/session.v4.jsonl");

#[test]
fn parses_v4_dialogue_and_native_tool_results_without_replacement_copies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl");
    fs::write(&path, FIXTURE).unwrap();
    let session = DshProvider.parse_session_file(&path).unwrap();

    assert_eq!(session.id.as_deref(), Some("v4-session"));
    assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
    assert_eq!(session.title.as_deref(), Some("Run diagnostics."));
    assert_eq!(session.blocks.len(), 8);
    let dialogue: Vec<_> = session
        .blocks
        .iter()
        .filter(|block| block.kind != AgentBlockKind::ToolCall)
        .map(|block| (block.kind, block.text.as_str()))
        .collect();
    assert_eq!(
        dialogue,
        vec![
            (AgentBlockKind::User, "Run diagnostics."),
            (AgentBlockKind::Thinking, "Check environment."),
            (AgentBlockKind::Assistant, "Running two checks."),
            (AgentBlockKind::ToolOutput, "READY"),
            (AgentBlockKind::ToolOutput, "Command failed: exit 1"),
            (
                AgentBlockKind::Assistant,
                "One check passed and one failed."
            ),
        ]
    );
    for (index, id, time) in [(5, "ok-call", "1006"), (6, "failed-call", "1008")] {
        let block = &session.blocks[index];
        assert_eq!(block.label.as_deref(), Some("bash"));
        assert_eq!(block.call_id.as_deref(), Some(id));
        assert_eq!(block.timestamp.as_deref(), Some(time));
    }
    assert_eq!(select_blocks(&session, AgentSelection::LastTurn).len(), 8);
}

#[test]
fn v4_metadata_uses_only_append_origin_human_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl");
    let header = json!({"type":"session", "version":4, "id":"v4", "cwd":"C:\\repo"});
    let replaced = json!({"type":"user/message", "seq":1, "time":1,
        "data":{"role":"user", "id":"copy", "source":{"kind":"user"},
            "content":[{"type":"text","text":"Replacement is not the title"}]},
        "surfaceOp":{"op":"replace","startSeq":1,"endSeq":1}});
    let prompt = json!({"type":"user/message", "seq":2, "time":2,
        "data":{"role":"user", "id":"prompt", "source":{"kind":"user"},
            "content":[{"type":"text","text":"Original prompt"}]}, "surfaceOp":"append"});
    fs::write(&path, format!("{header}\n{replaced}\n{prompt}\n")).unwrap();
    let meta = parse_session_meta(&path).unwrap();
    assert_eq!(meta.title.as_deref(), Some("Original prompt"));
}

#[test]
fn skips_native_producer_context_without_losing_v4_dialogue_or_title() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl");
    let log = FIXTURE.replace(
        "\"kind\":\"agent-instructions\"",
        "\"kind\":\"example-context\"",
    );
    fs::write(&path, log).unwrap();

    let meta = parse_session_meta(&path).unwrap();
    assert_eq!(meta.title.as_deref(), Some("Run diagnostics."));
    let session = DshProvider.parse_session_file(&path).unwrap();
    assert_eq!(session.title.as_deref(), Some("Run diagnostics."));
    assert_eq!(session.blocks.len(), 8);
    assert_eq!(session.blocks[0].text, "Run diagnostics.");
    assert!(!session
        .blocks
        .iter()
        .any(|block| block.text.contains("Injected workspace instructions")));
}

#[test]
fn rejects_retired_plugin_user_sources_in_v4_metadata_and_dialogue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl");
    let log = FIXTURE.replace(
        "\"source\":{\"kind\":\"agent-instructions\"}",
        "\"source\":{\"kind\":\"plugin\",\"plugin\":\"@example/context\",\"form\":\"snapshot\"}",
    );
    fs::write(&path, log).unwrap();

    for error in [
        parse_session_meta(&path).unwrap_err(),
        DshProvider.parse_session_file(&path).unwrap_err(),
    ] {
        let error = format!("{error:#}");
        assert!(error.contains("native source attribution"), "{error}");
        assert!(error.contains("line 2"), "{error}");
    }
}

#[test]
fn rejects_v4_messages_without_native_source_and_tool_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl");
    let fixture: Vec<Value> = FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (index, mutate) in [
        (2, 0), // Missing direct-user provenance.
        (4, 1), // Retired plugin source on an assistant message.
        (6, 2), // Tool result linked to a different invocation.
        (6, 3), // Missing native tool role.
        (8, 4), // Error metadata contradicts isError.
    ] {
        let mut rows = fixture.clone();
        match mutate {
            0 => {
                rows[index]["data"]
                    .as_object_mut()
                    .unwrap()
                    .remove("source");
            }
            1 => rows[index]["data"]["message"]["source"]["kind"] = json!("plugin"),
            2 => rows[index]["data"]["message"]["toolCallId"] = json!("wrong-call"),
            3 => rows[index]["data"]["message"]["role"] = json!("user"),
            4 => rows[index]["data"]["message"]["isError"] = json!(false),
            _ => unreachable!(),
        }
        let log = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, log).unwrap();
        let error = DshProvider.parse_session_file(&path).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("v4"), "{error}");
        assert!(error.contains("line"), "{error}");
        if mutate == 0 {
            assert!(parse_session_meta(&path).is_err());
        }
    }
}

#[test]
fn discovers_v4_successor_once_without_falling_back_to_invalid_or_future_generation() {
    let _guard = crate::test_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("DSH_HOME");
    std::env::set_var("DSH_HOME", dir.path());
    let root = dir.path().join("sessions");
    let folder = root.join("--repo--").join("v4-session");
    fs::create_dir_all(&folder).unwrap();
    let legacy = folder.join("session.jsonl");
    fs::write(&legacy, tests::fixture_with_cwd("C:\\repo")).unwrap();
    // Warm the old discovery cache; a provider upgrade must rescan an unchanged directory.
    let legacy_listing = list_sessions_matching(
        PROVIDER_NAME,
        &root,
        None,
        |path, is_dir| !is_dir && path.file_name().is_some_and(|name| name == "session.jsonl"),
        parse_session_meta,
    )
    .unwrap();
    assert_eq!(legacy_listing.len(), 1);
    let successor = folder.join("session.v4.jsonl");
    fs::write(&successor, FIXTURE).unwrap();

    let listed = DshProvider.list_recent_sessions(None).unwrap();
    fs::write(&successor, "not json\n").unwrap();
    let invalid = DshProvider.list_recent_sessions(None).unwrap();
    fs::write(&successor, FIXTURE).unwrap();
    fs::write(
        folder.join("session.v5.jsonl"),
        FIXTURE.replace("\"version\":4", "\"version\":5"),
    )
    .unwrap();
    let future = DshProvider.list_recent_sessions(None).unwrap();
    match previous {
        Some(value) => std::env::set_var("DSH_HOME", value),
        None => std::env::remove_var("DSH_HOME"),
    }
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, successor);
    assert_eq!(listed[0].id.as_deref(), Some("v4-session"));
    assert!(invalid.is_empty());
    assert!(future.is_empty());
}

#[test]
fn parses_v4_zstd_frames_and_retains_complete_events_before_a_torn_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl.zstd");
    let mut bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dsh/session.v4.jsonl.zstd"),
    )
    .unwrap();
    bytes.extend_from_slice(&[0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x01]);
    fs::write(&path, bytes).unwrap();
    let session = DshProvider.parse_session_file(&path).unwrap();
    assert_eq!(session.blocks.len(), 8);
    assert_eq!(session.blocks[5].text, "READY");
    assert_eq!(session.blocks[6].text, "Command failed: exit 1");
}

#[test]
fn refuses_unknown_required_v4_events_but_accepts_explicitly_ignorable_trace() {
    let path = Path::new("session.v4.jsonl");
    let unknown = json!({"type":"future/history-rewrite", "seq":13, "time":1013, "data":{}});
    let error = parse_log_text(path, &format!("{FIXTURE}{unknown}\n")).unwrap_err();
    assert!(format!("{error:#}").contains("unsupported Dsh v4 event type"));

    let mut trace = unknown;
    trace["ignorable"] = json!(true);
    let session = parse_log_text(path, &format!("{FIXTURE}{trace}\n")).unwrap();
    assert_eq!(session.blocks.len(), 8);
}

#[test]
fn rejects_a_v4_filename_with_a_legacy_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.v4.jsonl");
    fs::write(&path, tests::fixture_with_cwd("C:\\repo")).unwrap();
    assert!(DshProvider.parse_session_file(&path).is_err());
    assert!(parse_session_meta(&path).is_err());
}

#[test]
fn lists_compressed_v4_once_when_plain_and_legacy_logs_coexist() {
    let _guard = crate::test_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("DSH_HOME");
    std::env::set_var("DSH_HOME", dir.path());
    let folder = dir
        .path()
        .join("sessions")
        .join("--repo--")
        .join("v4-session");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("session.jsonl"),
        tests::fixture_with_cwd("C:\\repo"),
    )
    .unwrap();
    fs::write(folder.join("session.v4.jsonl"), FIXTURE).unwrap();
    let compressed = folder.join("session.v4.jsonl.zstd");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dsh/session.v4.jsonl.zstd"),
        &compressed,
    )
    .unwrap();
    let listed = DshProvider.list_recent_sessions(None).unwrap();
    match previous {
        Some(value) => std::env::set_var("DSH_HOME", value),
        None => std::env::remove_var("DSH_HOME"),
    }
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, compressed);
    assert_eq!(listed[0].title.as_deref(), Some("Run diagnostics."));
}
