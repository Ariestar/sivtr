use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use fs2::FileExt;
use sivtr::remote::protocol::DaemonInfo;

/// Active daemon errors must reach the CLI without deleting its control file.
#[test]
fn daemon_errors_preserve_control_file() {
    for action in ["stop", "start", "restart", "status"] {
        for response in [
            "{\"Error\":{\"message\":\"test daemon error\"}}\n",
            "invalid json\n",
            "\"Ok\"\n",
            "",
        ] {
            if response.is_empty() && action != "stop" {
                continue;
            }
            let home = tempfile::tempdir().expect("create isolated daemon home");
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind control listener");
            listener
                .set_nonblocking(true)
                .expect("set nonblocking listener");
            let lock = std::fs::File::create(home.path().join("daemon.lock")).expect("create lock");
            lock.lock_exclusive().expect("hold daemon lock");
            let node = iroh::SecretKey::from_bytes(&[7; 32]).public();
            let info = DaemonInfo {
                pid: std::process::id(),
                port: listener.local_addr().expect("listener address").port(),
                token: "test-token".to_string(),
                node_id: node.to_string(),
                endpoint: iroh::EndpointAddr::new(node),
                started_at: "2026-10-29T00:00:00Z".to_string(),
            };
            let path = home.path().join("daemon.json");
            let original = serde_json::to_vec(&info).expect("serialize daemon info");
            std::fs::write(&path, &original).expect("write daemon info");
            let (stop, stopped) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let mut requests = 0;
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_nonblocking(false)
                                .expect("blocking control stream");
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .expect("set read timeout");
                            let mut request = String::new();
                            BufReader::new(&mut stream)
                                .read_line(&mut request)
                                .expect("read status request");
                            requests += 1;
                            if response.is_empty() {
                                stopped
                                    .recv_timeout(Duration::from_secs(40))
                                    .expect("command timed out");
                                break requests;
                            }
                            stream
                                .write_all(response.as_bytes())
                                .expect("write response");
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => panic!("accept control request: {error}"),
                    }
                    if stopped.recv_timeout(Duration::from_millis(10)).is_ok() {
                        break requests;
                    }
                }
            });
            let output = Command::new(env!("CARGO_BIN_EXE_sivtr"))
                .args(["serve", action])
                .env("SIVTR_HOME", home.path())
                .output()
                .expect("run daemon command");
            stop.send(()).expect("stop control listener");
            assert!(
                server.join().expect("join control listener") > 0,
                "must reach the daemon"
            );
            assert!(
                !output.status.success(),
                "{action}: must report the daemon error"
            );
            assert_eq!(
                std::fs::read(&path).expect("preserve control file"),
                original,
                "{action}"
            );
        }
    }
}

/// Missing or malformed state remains removable when no daemon holds the lock.
#[test]
fn stopped_daemon_allows_stale_control_cleanup() {
    for contents in [None, Some("invalid json")] {
        let home = tempfile::tempdir().expect("isolated home");
        let path = home.path().join("daemon.json");
        std::fs::File::create(home.path().join("daemon.lock")).expect("stale lock");
        if let Some(contents) = contents {
            std::fs::write(&path, contents).expect("stale control file");
        }
        let output = Command::new(env!("CARGO_BIN_EXE_sivtr"))
            .args(["serve", "stop"])
            .env("SIVTR_HOME", home.path())
            .output()
            .expect("stop stale daemon");
        assert!(output.status.success(), "{output:?}");
        assert!(!path.exists());
    }
}

/// Shutdown must wait for lock release without sending another control request.
#[test]
fn shutdown_waits_for_lock_release_without_another_status_request() {
    use sivtr::remote::protocol::{
        DaemonStatus, LocalEnvelope, LocalRequest, LocalResponse, PROTOCOL_VERSION,
    };
    let home = tempfile::tempdir().expect("isolated home");
    let listener = TcpListener::bind("127.0.0.1:0").expect("control listener");
    let lock = std::fs::File::create(home.path().join("daemon.lock")).expect("lock file");
    lock.lock_exclusive().expect("hold lock");
    let node = iroh::SecretKey::from_bytes(&[7; 32]).public();
    let info = DaemonInfo {
        pid: std::process::id(),
        port: listener.local_addr().expect("address").port(),
        token: "test-token".into(),
        node_id: node.to_string(),
        endpoint: iroh::EndpointAddr::new(node),
        started_at: "2026-10-29T00:00:00Z".into(),
    };
    std::fs::write(
        home.path().join("daemon.json"),
        serde_json::to_vec(&info).expect("info"),
    )
    .expect("write info");
    let server = std::thread::spawn(move || {
        for shutdown in [false, true] {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .expect("read request");
            let request: LocalEnvelope = serde_json::from_str(&line).expect("request");
            let response = if shutdown {
                assert!(matches!(request.request, LocalRequest::Shutdown));
                LocalResponse::Ok
            } else {
                assert!(matches!(request.request, LocalRequest::Status));
                LocalResponse::Status(DaemonStatus {
                    node_id: info.node_id.clone(),
                    device_name: "test".into(),
                    endpoint: info.endpoint.clone(),
                    started_at: info.started_at.clone(),
                    shares: 0,
                    peers: 0,
                    protocol_version: PROTOCOL_VERSION,
                })
            };
            serde_json::to_writer(&mut stream, &response).expect("response");
            stream.write_all(b"\n").expect("newline");
        }
        // Match endpoint cleanup: keep the listener bound but stop accepting.
        std::thread::sleep(Duration::from_millis(250));
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "shutdown must not send another status request"
        );
        drop(lock);
        drop(listener);
    });
    let output = Command::new(env!("CARGO_BIN_EXE_sivtr"))
        .args(["serve", "stop"])
        .env("SIVTR_HOME", home.path())
        .output()
        .expect("stop daemon");
    server.join().expect("server");
    assert!(output.status.success(), "{output:?}");
}
