use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::model::{
    workspace_matches_candidates, AgentSession, AgentSessionInfo, AgentSessionMeta,
    WorkspaceMatchTarget,
};

pub fn list_recent_jsonl_sessions(
    root: &Path,
    cwd: Option<&Path>,
    parse_meta: impl Fn(&Path) -> Result<AgentSessionMeta>,
) -> Result<Vec<AgentSessionInfo>> {
    let wanted = cwd.map(WorkspaceMatchTarget::new);
    let mut sessions = Vec::new();

    for path in jsonl_files(root)? {
        let meta = match parse_meta(&path) {
            Ok(meta) => meta,
            Err(error) => {
                eprintln!(
                    "warning: failed to parse agent session metadata {}: {error:#}",
                    path.display()
                );
                continue;
            }
        };
        // Shared policy: no cwd metadata → keep; otherwise path or git-remote match.
        if let Some(wanted) = wanted.as_ref() {
            if !workspace_matches_candidates(wanted, meta.cwd_candidates().map(Path::new)) {
                continue;
            }
        }

        sessions.push(AgentSessionInfo {
            modified: modified_time(&path).unwrap_or(SystemTime::UNIX_EPOCH),
            path,
            id: meta.id,
            cwd: meta.cwd,
            title: meta.title,
        });
    }

    sessions.sort_by_key(|session| session.modified);
    sessions.reverse();
    Ok(sessions)
}

pub fn parse_jsonl_session(
    path: &Path,
    provider_name: &str,
    mut apply_event: impl FnMut(&mut AgentSession, &Value),
) -> Result<AgentSession> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read {provider_name} session: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: None,
        cwd: None,
        title: None,
        blocks: Vec::new(),
    };

    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "Failed to read {provider_name} session line {}: {}",
                idx + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) if idx > 0 && is_trailing_partial_json_line(&error) => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to parse {provider_name} session line {} as JSON: {}",
                        idx + 1,
                        path.display()
                    )
                });
            }
        };
        apply_event(&mut session, &value);
    }

    Ok(session)
}

pub fn parse_jsonl_meta(
    path: &Path,
    provider_name: &str,
    max_lines: usize,
    mut update_meta: impl FnMut(&mut AgentSessionMeta, &Value),
) -> Result<AgentSessionMeta> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read {provider_name} session: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut meta = AgentSessionMeta::default();

    for (idx, line) in reader.lines().take(max_lines).enumerate() {
        let line = line.with_context(|| {
            format!(
                "Failed to read {provider_name} session metadata line {}: {}",
                idx + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse {provider_name} session metadata as JSON: {}",
                path.display()
            )
        })?;
        update_meta(&mut meta, &value);
    }

    Ok(meta)
}

pub fn jsonl_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files)?;
    Ok(files)
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn is_trailing_partial_json_line(error: &serde_json::Error) -> bool {
    matches!(error.classify(), serde_json::error::Category::Eof)
}

fn modified_time(path: &Path) -> Result<SystemTime> {
    Ok(fs::metadata(path)?.modified()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_git_remote(repo: &Path, name: &str, url: &str) {
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git").join("config"),
            format!("[remote \"{name}\"]\n\turl = {url}\n"),
        )
        .unwrap();
    }

    #[test]
    fn includes_sessions_with_later_matching_cwd_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let target = dir.path().join("oh-my-ppt-fork");
        let candidate = dir.path().join("oh-my-ppt");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        write_git_remote(
            &target,
            "upstream",
            "https://github.com/arcsin1/oh-my-ppt.git",
        );
        write_git_remote(
            &candidate,
            "origin",
            "https://github.com/arcsin1/oh-my-ppt.git",
        );
        let transcript = sessions.join("session.jsonl");
        let first_event = serde_json::json!({
            "sessionId": "abc",
            "cwd": dir.path(),
            "customTitle": "Initial",
        });
        let second_event = serde_json::json!({
            "sessionId": "abc",
            "cwd": candidate,
        });
        fs::write(&transcript, format!("{first_event}\n{second_event}\n")).unwrap();

        let sessions = list_recent_jsonl_sessions(&sessions, Some(&target), |path| {
            parse_jsonl_meta(path, "Claude", 50, |meta, value| {
                if meta.id.is_none() {
                    meta.id = value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    meta.add_cwd(cwd);
                }
                if meta.title.is_none() {
                    meta.title = value
                        .get("customTitle")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
            })
        })
        .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("abc"));
        assert_eq!(
            sessions[0].cwd.as_deref(),
            Some(dir.path().to_str().unwrap())
        );
    }

    #[test]
    fn keeps_sessions_without_cwd_when_filtering_by_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let target = dir.path().join("repo");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&target).unwrap();

        let no_cwd = sessions.join("no-cwd.jsonl");
        fs::write(
            &no_cwd,
            r#"{"role":"session_meta","platform":"weixin","model":"m"}
{"role":"user","content":"hi"}
"#,
        )
        .unwrap();

        let wrong_cwd = sessions.join("wrong-cwd.jsonl");
        let other = dir.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(
            &wrong_cwd,
            format!(
                "{}\n",
                serde_json::json!({
                    "sessionId": "wrong",
                    "cwd": other,
                })
            ),
        )
        .unwrap();

        let listed = list_recent_jsonl_sessions(&sessions, Some(&target), |path| {
            parse_jsonl_meta(path, "Hermes", 5, |meta, value| {
                if meta.id.is_none() {
                    meta.id = value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            path.file_stem()
                                .and_then(|name| name.to_str())
                                .map(str::to_string)
                        });
                }
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    meta.add_cwd(cwd);
                }
            })
        })
        .unwrap();

        let ids: Vec<_> = listed
            .iter()
            .filter_map(|session| session.id.clone())
            .collect();
        assert!(ids.iter().any(|id| id == "no-cwd"));
        assert!(!ids.iter().any(|id| id == "wrong"));
    }
}
