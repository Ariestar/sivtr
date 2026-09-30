use super::*;
use crate::agents::{select_blocks, AgentSelection};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_env_lock()
}

/// Compact but realistic dsh log: packed chunk rows are present but must
/// not produce blocks; injected user/messages (workspace instructions,
/// system-prompt snapshots) must be skipped. `{{cwd}}` is substituted per
/// test so Windows backslashes stay JSON-escaped.
const FIXTURE: &str = r#"{"type":"session","version":0,"id":"ff1c1e99-3bd4-4ef8-a954-80d607d628ba","createdAt":1783352165190,"cwd":"{{cwd}}","delegationDepth":0}
{"type":"user/message","seq":4,"time":1785498765364,"data":{"content":[{"type":"text","text":"Use the bash tool to run exactly: echo HELLO. Report the tool result you got back verbatim, then stop."}],"source":{"kind":"user"},"role":"user","id":"a207bd9d-9312-46ed-baaf-7a07a6f08ae8"},"surfaceOp":"append"}
{"type":"user/message","seq":5,"time":1785730418683,"data":{"content":[{"type":"text","text":"Instructions from: AGENTS.md\n\nThis file is the single source of truth."}],"source":{"kind":"agent-instructions"},"role":"user","id":"1c954f81-4e70-4e28-bf11-5f8424f09391"},"surfaceOp":"append"}
{"type":"session/title","seq":6,"time":1785730418683,"data":{"title":"Use the bash tool to","messageSeqs":[4],"source":{"kind":"fallback"}}}
{"type":"assistant/chunk","seq":9,"time":1783352166048,"data":{"turn":1,"step":1,"chunk":{"type":"block-start","index":0,"blockType":"reasoning"}}}
{"type":"reasoning-chunks","seq0":10,"time0":1783352166075,"data":{"turn":1,"step":1,"index":0,"dt":[0,0,1,0,0,28],"texts":["The"," user"," wants"," me"," to"," run"]}}
{"type":"assistant/chunk","seq":27,"time":1783352166250,"data":{"turn":1,"step":1,"chunk":{"type":"block-start","index":1,"blockType":"tool-call"}}}
{"type":"tool-call-chunks","seq0":28,"time0":1783352166250,"data":{"turn":1,"step":1,"index":1,"dt":[0,28,1],"id":"call_00_JliP571Bh0QQ8QExbSPk0080","name":"bash","args":["{","\"","command"]}}
{"type":"assistant/message","seq":57,"time":1785730418696,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"reasoning","text":"The user wants me to run a simple bash command and report the result verbatim."},{"type":"tool-call","id":"call_00_JliP571Bh0QQ8QExbSPk0080","name":"bash","arguments":"{\"command\": \"echo HELLO\", \"description\": \"Run echo HELLO\"}"}],"source":{"kind":"model","provider":"deepseek-official","model":"deepseek-v4-flash"},"id":"658eb4a4-7462-43d8-91eb-13d09363db20"}},"sourceEventSeqs":[9,10,27,28],"surfaceOp":"append"}
{"type":"tool/call","seq":58,"time":1785730418696,"data":{"turn":1,"step":1,"callId":"call_00_JliP571Bh0QQ8QExbSPk0080","name":"bash","arguments":"{\"command\": \"echo HELLO\", \"description\": \"Run echo HELLO\"}"}}
{"type":"tool/result","seq":61,"time":1785730418702,"data":{"turn":1,"step":1,"message":{"source":{"kind":"tool","callId":"call_00_JliP571Bh0QQ8QExbSPk0080"},"content":[{"type":"tool-result","toolCallId":"call_00_JliP571Bh0QQ8QExbSPk0080","content":[{"type":"text","text":"Error: bash is disabled by policy in this session"}],"isError":true}],"role":"user","id":"85f289f4-cb3c-468e-bbad-e66fefe2346f"}},"sourceEventSeqs":[58],"surfaceOp":"append"}
{"type":"text-chunks","seq0":87,"time0":1783352167672,"data":{"turn":1,"step":2,"index":1,"dt":[29,0,1],"texts":["The"," tool"," returned"]}}
{"type":"assistant/message","seq":121,"time":1785730418716,"data":{"turn":1,"step":2,"message":{"role":"assistant","content":[{"type":"text","text":"The tool returned:\n\n> Error: bash is disabled by policy in this session\n\nI cannot run the command because the bash tool is disabled by policy."}],"source":{"kind":"model","provider":"deepseek-official","model":"deepseek-v4-flash"},"id":"0bea7b77-242e-4399-bd10-90324a37fff0"}},"sourceEventSeqs":[87,88],"surfaceOp":"append"}
{"type":"turn/end","seq":123,"time":1785730418717,"data":{"turn":1,"reason":{"kind":"completed"}}}
"#;

/// Substitute the fixture's cwd placeholder with a JSON-escaped path.
pub(super) fn fixture_with_cwd(cwd: &str) -> String {
    FIXTURE.replace("{{cwd}}", &cwd.replace('\\', "\\\\"))
}

#[test]
fn parses_dsh_session_blocks_from_surface_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, fixture_with_cwd("C:\\repo")).unwrap();

    let session = DshProvider.parse_session_file(&path).unwrap();

    assert_eq!(
        session.id.as_deref(),
        Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
    );
    assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
    assert_eq!(session.title.as_deref(), Some("Use the bash tool to"));
    // user, thinking, tool-call, tool-output, assistant. The injected
    // message, chunk rows, and tool/call must not add blocks.
    assert_eq!(session.blocks.len(), 5);
    assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
    assert_eq!(session.blocks[0].text, "Use the bash tool to run exactly: echo HELLO. Report the tool result you got back verbatim, then stop.");
    assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
    assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
    assert_eq!(session.blocks[2].label.as_deref(), Some("bash"));
    assert!(session.blocks[2]
        .text
        .contains("\"command\": \"echo HELLO\""));
    assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
    assert_eq!(session.blocks[3].label.as_deref(), Some("bash"));
    assert_eq!(
        session.blocks[3].text,
        "Error: bash is disabled by policy in this session"
    );
    assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
    assert!(session.blocks[4].text.contains("I cannot run the command"));
}

#[test]
fn only_user_sourced_messages_become_user_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"session","version":0,"id":"s1","cwd":"C:\\repo","delegationDepth":0}
{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"direct prompt"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
{"type":"user/message","seq":2,"time":2000,"data":{"content":[{"type":"text","text":"workspace instructions"}],"source":{"kind":"agent-instructions"},"role":"user","id":"m2"},"surfaceOp":"append"}
{"type":"user/message","seq":3,"time":3000,"data":{"content":[{"type":"text","text":"skill catalog"}],"source":{"kind":"skill-catalog"},"role":"user","id":"m3"},"surfaceOp":"append"}
{"type":"user/message","seq":4,"time":4000,"data":{"content":[{"type":"text","text":"runtime snapshot"}],"source":{"kind":"plugin","plugin":"@deepseek-ai/dsh-system-prompt","form":"snapshot"},"role":"user","id":"m4"},"surfaceOp":"append"}
{"type":"user/message","seq":5,"time":5000,"data":{"content":[{"type":"text","text":"goal continuation"}],"source":{"kind":"goal"},"role":"user","id":"m5"},"surfaceOp":"append"}
{"type":"assistant/message","seq":6,"time":6000,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"text","text":"done"}],"source":{"kind":"model","provider":"deepseek-official"},"id":"a1"}},"surfaceOp":"append"}
"#,
    )
    .unwrap();

    let session = DshProvider.parse_session_file(&path).unwrap();

    let user_blocks: Vec<_> = session
        .blocks
        .iter()
        .filter(|block| block.kind == AgentBlockKind::User)
        .collect();
    assert_eq!(user_blocks.len(), 1);
    assert_eq!(user_blocks[0].text, "direct prompt");
    assert_eq!(session.title.as_deref(), Some("direct prompt"));
}

#[test]
fn fallback_title_uses_first_user_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"session","version":0,"id":"s1","cwd":"C:\\repo","delegationDepth":0}
{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"Fix the flaky test"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
{"type":"assistant/message","seq":2,"time":2000,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"text","text":"On it."}],"source":{"kind":"model","provider":"deepseek-official"},"id":"a1"}},"surfaceOp":"append"}
"#,
    )
    .unwrap();

    let session = DshProvider.parse_session_file(&path).unwrap();

    assert_eq!(session.title.as_deref(), Some("Fix the flaky test"));
    assert_eq!(session.blocks.len(), 2);
    assert_eq!(session.blocks[0].text, "Fix the flaky test");
    assert_eq!(session.blocks[1].text, "On it.");
}

#[test]
fn refuses_foreign_session_format_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"session","version":1,"id":"future","cwd":"C:\\repo","delegationDepth":0}
{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"hi"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
"#,
    )
    .unwrap();

    let error = DshProvider.parse_session_file(&path).unwrap_err();
    assert!(format!("{error:#}").contains("unsupported Dsh session log version"));
}

#[test]
fn refuses_log_without_session_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"hi"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
"#,
    )
    .unwrap();

    let error = DshProvider.parse_session_file(&path).unwrap_err();
    assert!(format!("{error:#}").contains("missing session header"));
}

#[test]
fn refuses_empty_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, "").unwrap();

    let error = DshProvider.parse_session_file(&path).unwrap_err();
    assert!(format!("{error:#}").contains("missing session header"));
}

#[test]
fn refuses_empty_zstd_log() {
    // A .zstd artifact with no complete frame decodes to nothing, which
    // must be rejected like an empty plain log.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl.zstd");
    std::fs::write(&path, []).unwrap();

    let error = DshProvider.parse_session_file(&path).unwrap_err();
    assert!(format!("{error:#}").contains("missing session header"));
}

#[test]
fn lists_dsh_sessions_under_dsh_home() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("DSH_HOME");
    std::env::set_var("DSH_HOME", dir.path());

    let sessions = dir.path().join("sessions").join("--repo--").join("s1");
    std::fs::create_dir_all(&sessions).unwrap();
    let repo_cwd = dir.path().join("repo");
    std::fs::write(
        sessions.join("session.jsonl"),
        fixture_with_cwd(&repo_cwd.to_string_lossy()),
    )
    .unwrap();
    // Malformed sibling must be skipped during listing.
    let bad = dir.path().join("sessions").join("--repo--").join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("session.jsonl"), "{not json}\n").unwrap();
    // Empty sibling must be skipped during listing, not listed as unbound.
    let empty = dir.path().join("sessions").join("--repo--").join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("session.jsonl"), "").unwrap();

    let listed = DshProvider.list_recent_sessions(None).unwrap();

    match previous {
        Some(value) => std::env::set_var("DSH_HOME", value),
        None => std::env::remove_var("DSH_HOME"),
    }

    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].id.as_deref(),
        Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
    );
    assert_eq!(
        listed[0].cwd.as_deref(),
        Some(repo_cwd.to_string_lossy().as_ref())
    );
    assert_eq!(listed[0].title.as_deref(), Some("Use the bash tool to"));
}

#[test]
fn filters_dsh_sessions_by_workspace() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("DSH_HOME");
    std::env::set_var("DSH_HOME", dir.path());

    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let in_repo = dir.path().join("sessions").join("--repo--").join("s1");
    std::fs::create_dir_all(&in_repo).unwrap();
    std::fs::write(
        in_repo.join("session.jsonl"),
        fixture_with_cwd(&repo.to_string_lossy()),
    )
    .unwrap();
    let other = dir.path().join("sessions").join("--elsewhere--").join("s2");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("session.jsonl"),
        fixture_with_cwd(&dir.path().join("elsewhere").to_string_lossy()),
    )
    .unwrap();

    let listed = DshProvider.list_recent_sessions(Some(&repo)).unwrap();

    match previous {
        Some(value) => std::env::set_var("DSH_HOME", value),
        None => std::env::remove_var("DSH_HOME"),
    }

    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].id.as_deref(),
        Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
    );
}

#[test]
fn decodes_zstd_concat_frames_and_torn_tail() {
    let bytes = zstd_fixture();

    let decoded = decode_zstd(&bytes, None).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), FIXTURE);

    // A torn trailing frame must be dropped, keeping the complete frames.
    let mut torn = bytes.clone();
    torn.extend_from_slice(&[0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x01]);
    let decoded = decode_zstd(&torn, None).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), FIXTURE);
}

#[test]
fn parses_zstd_session_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl.zstd");
    std::fs::write(&path, zstd_fixture()).unwrap();

    let session = DshProvider.parse_session_file(&path).unwrap();

    assert_eq!(
        session.id.as_deref(),
        Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
    );
    assert_eq!(session.blocks.len(), 5);
    assert_eq!(select_blocks(&session, AgentSelection::LastTurn).len(), 5);
}

/// dsh writes `session.jsonl.zstd` as one checksummed zstd frame per flush
/// batch (header frame first). The fixture is committed as plaintext;
/// compress the header line and the events as two separate frames, as the
/// backend would.
fn zstd_fixture() -> Vec<u8> {
    let fixture_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dsh/session.jsonl.zstd");
    fs::read(&fixture_path)
        .unwrap_or_else(|_| panic!("missing zstd fixture: {}", fixture_path.display()))
}
