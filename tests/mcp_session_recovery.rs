//! Exercise real stdio MCP with invented, fully materialized WorkSets.
//! Saved-set queries avoid native transcript discovery and archive sync.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sivtr_core::workset::WorkSet;

const FIXTURE: &str = include_str!("fixtures/mcp-session-recovery.json");
const STALE: &str = "codex/synthetic-retry-policy-old/1";
const UNRELATED: &str = "codex/synthetic-color-theme/1";
const CURRENT: [&str; 3] = [
    "codex/synthetic-retry-policy/3",
    "codex/synthetic-retry-policy/2",
    "codex/synthetic-retry-policy/1",
];

struct McpServer {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Result<Value, String>>,
    next_id: u64,
}

impl McpServer {
    fn start(data: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sivtr"))
            .args(["mcp", "serve", "--idle-exit", "0"])
            .current_dir(data)
            .env("SIVTR_DATA_DIR", data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start stdio MCP server");
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("capture MCP stdout");
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            input,
            output,
            next_id: 1,
        };
        let initialized = server.request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "synthetic-recovery-check", "version": "1.0.0"}
            }),
        );
        assert!(initialized["capabilities"]["tools"].is_object());
        server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        server
    }

    fn send(&mut self, message: &Value) {
        let input = self.input.as_mut().expect("MCP stdin is open");
        writeln!(input, "{message}").expect("write MCP request");
        input.flush().expect("flush MCP request");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if method == "tools/call" {
            println!("MCP request: {request}");
        }
        self.send(&request);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let response = self
                .output
                .recv_timeout(remaining)
                .expect("MCP response within 15 seconds")
                .expect("MCP stdout contains JSON");
            if response.get("id").is_none() {
                continue;
            }
            assert_eq!(response["jsonrpc"], "2.0");
            assert_eq!(response["id"], id);
            assert!(response.get("error").is_none(), "MCP error: {response}");
            return response["result"].clone();
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        assert_ne!(result["isError"], true, "tool failed: {result}");
        let content = result["content"].as_array().expect("MCP content array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        let decoded: Value =
            serde_json::from_str(content[0]["text"].as_str().expect("text result"))
                .expect("decode sivtr JSON result");
        println!("Decoded result.content[0].text: {decoded}");
        decoded
    }

    fn close(mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().expect("check MCP exit") {
                assert!(status.success(), "MCP exited with {status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "MCP did not exit after stdin closed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        // Also reap the child when an assertion or a response timeout fails.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn recover_cited_evidence_after_mcp_restart() {
    let data = tempfile::tempdir().expect("create isolated fixture directory");
    let fixture: WorkSet = serde_json::from_str(FIXTURE).expect("decode synthetic WorkSet");
    fixture.validate().expect("validate synthetic WorkSet");
    assert_eq!(fixture.records().len(), 5);
    assert!(fixture
        .records()
        .iter()
        .all(|record| { !record.parts.is_empty() && record.session.path.is_none() }));
    let sets = data.path().join("sets");
    std::fs::create_dir(&sets).expect("create isolated WorkSet store");
    std::fs::write(sets.join("fixture.json"), FIXTURE).expect("seed synthetic WorkSet");

    let mut server = McpServer::start(data.path());
    let listed = server.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("list MCP tools");
    for name in ["sivtr_search", "sivtr_filter", "sivtr_show"] {
        assert!(tools.iter().any(|tool| tool["name"] == name));
    }

    let searched = server.call(
        "sivtr_search",
        json!({
            "source": "@fixture", "query": "retry-policy", "match_regex": "retry-policy",
            "limit": 10, "save": "recovery", "detail": "timeline"
        }),
    );
    assert_eq!(searched["count"], 4);
    assert_eq!(searched["saved_as"], "recovery");
    let hits = searched["anchors"]
        .as_array()
        .expect("returned search anchors");
    assert!(hits.contains(&json!(STALE)));
    assert!(!hits.contains(&json!(UNRELATED)));
    for reference in CURRENT {
        assert!(hits.contains(&json!(reference)));
    }
    for item in searched["items"].as_array().expect("timeline provenance") {
        assert!(hits.contains(&item["ref"]));
        assert_eq!(item["source"], "codex");
        assert!(item["time"]
            .as_str()
            .expect("record timestamp")
            .starts_with("2026-01-0"));
    }

    let current = server.call(
        "sivtr_filter",
        json!({
            "source": "@recovery", "since": "2026-01-02T00:00:00Z",
            "save": "current", "detail": "workset"
        }),
    );
    assert_eq!(current["count"], 3);
    assert_eq!(current["saved_as"], "current");
    assert_eq!(current["anchors"], json!(CURRENT));
    let selected: WorkSet =
        serde_json::from_value(current["workset"].clone()).expect("decode returned WorkSet");
    selected
        .validate()
        .expect("returned anchors have backing records");
    assert_eq!(selected.records().len(), 3);
    assert_eq!(current["workset"]["anchors"], current["anchors"]);
    for record in selected.records() {
        assert_eq!(
            record.session.canonical_id.as_deref(),
            Some("synthetic-retry-policy")
        );
        assert!(record
            .time
            .started_at
            .as_deref()
            .expect("record time")
            .starts_with("2026-01-02"));
    }
    let verification = current["workset"]["records"]
        .as_array()
        .expect("materialized evidence")
        .iter()
        .find(|record| record["work_ref"] == CURRENT[1])
        .expect("selected verification record");
    assert_eq!(verification["status"]["outcome"], "success");
    assert_eq!(verification["status"]["exit_code"], 0);
    assert_eq!(verification["parts"][0]["kind"], "action");
    assert_eq!(verification["parts"][0]["exit_code"], 0);

    let unrelated = server.call(
        "sivtr_search",
        json!({
            "source": "@fixture", "match_regex": "color-theme", "limit": 1, "detail": "timeline"
        }),
    );
    assert_eq!(unrelated["anchors"], json!([UNRELATED]));
    assert_eq!(unrelated["items"][0]["time"], "2026-01-03T10:00:00Z");
    let last = server.call("sivtr_show", json!({"source": "@last", "mode": "refs"}));
    assert_eq!(last["anchors"], unrelated["anchors"]);
    server.close();

    assert!(sets.join("last.json").is_file());
    assert!(sets.join("current.json").is_file());
    let mut resumed = McpServer::start(data.path());
    for (reference, evidence) in [
        (CURRENT[2], "cap retries at 2; keep exponential backoff"),
        (CURRENT[1], "3 passed; 0 failed"),
        (CURRENT[0], "add a cancellation case"),
    ] {
        let position = current["anchors"]
            .as_array()
            .expect("saved anchor order")
            .iter()
            .position(|anchor| anchor == reference)
            .expect("choose returned WorkRef")
            + 1;
        let shown = resumed.call(
            "sivtr_show",
            json!({
                "source": format!("@current[{position}]"), "mode": "full"
            }),
        );
        assert_eq!(shown["count"], 1);
        assert_eq!(shown["anchors"], json!([reference]));
        assert_eq!(shown["contents"][0]["ref"], reference);
        let content = shown["contents"][0]["content"]
            .as_str()
            .expect("expanded evidence");
        assert!(content.contains(evidence));
        if reference == CURRENT[1] {
            assert!(content.contains("cargo test retry_policy --locked"));
        }
        assert!(!content.contains("proposed retry cap was 5"));
        assert!(!content.contains("color-theme"));
        println!("Synthetic recovery evidence [{reference}]: {content}");
    }
    println!("Agent inference: resume by adding cancellation coverage; these saved synthetic results do not prove the current checkout passes tests.");
    resumed.close();
    assert!(
        !data.path().join("archive.db").exists(),
        "saved-set recovery must not sync native transcripts"
    );
}
