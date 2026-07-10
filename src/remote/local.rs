use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use sivtr_core::workspace;

use super::protocol::{DaemonInfo, LocalEnvelope, LocalRequest, LocalResponse};

pub fn daemon_info_path() -> PathBuf {
    workspace::data_dir().join("daemon.json")
}

pub fn daemon_lock_path() -> PathBuf {
    workspace::data_dir().join("daemon.lock")
}

pub fn daemon_log_path() -> PathBuf {
    workspace::data_dir().join("daemon.log")
}

pub fn read_daemon_info() -> Result<DaemonInfo> {
    let path = daemon_info_path();
    let text = std::fs::read_to_string(&path)
        .with_context(|| "sivtr daemon is not running; run `sivtr serve start`")?;
    serde_json::from_str(&text)
        .with_context(|| format!("Invalid daemon state at {}", path.display()))
}

pub fn write_daemon_info(info: &DaemonInfo) -> Result<()> {
    let path = daemon_info_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(info)?)?;
    restrict_file(&temporary)?;
    std::fs::rename(&temporary, &path)?;
    restrict_file(&path)?;
    Ok(())
}

pub fn remove_daemon_info() {
    let _ = std::fs::remove_file(daemon_info_path());
}

pub fn call(request: LocalRequest) -> Result<LocalResponse> {
    let info = read_daemon_info()?;
    call_with_info(&info, request)
}

pub fn call_with_info(info: &DaemonInfo, request: LocalRequest) -> Result<LocalResponse> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), info.port);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .with_context(|| "sivtr daemon is not responding; run `sivtr serve restart`")?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let envelope = LocalEnvelope {
        token: info.token.clone(),
        request,
    };
    serde_json::to_writer(&mut stream, &envelope)?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("Daemon closed the local control connection")?;
    let response: LocalResponse =
        serde_json::from_str(&line).context("Invalid response from sivtr daemon")?;
    match response {
        LocalResponse::Error { message } => Err(anyhow::anyhow!(message)),
        response => Ok(response),
    }
}

pub fn running() -> bool {
    let Ok(info) = read_daemon_info() else {
        return false;
    };
    matches!(
        call_with_info(&info, LocalRequest::Status),
        Ok(LocalResponse::Status(_))
    )
}

#[cfg(unix)]
fn restrict_file(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &std::path::Path) -> Result<()> {
    Ok(())
}
