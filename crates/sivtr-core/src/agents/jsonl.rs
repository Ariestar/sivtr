use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::model::{
    workspace_matches_candidates, AgentSession, AgentSessionInfo, AgentSessionMeta,
    WorkspaceMatchTarget,
};

/// Bump when the listing cache layout or meta parsing changes.
const LISTING_CACHE_VERSION: u32 = 1;

/// One cached session file in a listing: fingerprint + parsed metadata.
#[derive(Clone, Serialize, Deserialize)]
struct ListingEntry {
    mtime_secs: u64,
    mtime_nanos: u32,
    size: u64,
    meta: AgentSessionMeta,
}

/// Stamp-validated cache of a provider root's session listing.
///
/// Discovery walks the tree and stats every file on each call; only files
/// whose `(mtime, size)` fingerprint changed since the last run are opened
/// and meta-parsed, so steady-state discovery is a stat sweep instead of a
/// read + parse of every session file.
#[derive(Serialize, Deserialize, Default)]
struct ListingCache {
    version: u32,
    entries: HashMap<PathBuf, ListingEntry>,
}

pub fn list_recent_jsonl_sessions(
    provider: &str,
    root: &Path,
    cwd: Option<&Path>,
    parse_meta: impl Fn(&Path) -> Result<AgentSessionMeta>,
) -> Result<Vec<AgentSessionInfo>> {
    collect_recent_sessions(provider, root, cwd, jsonl_files(root)?, parse_meta)
}

/// Walk a chat-recording tmp root: `<tmp>/<project>*/chats/*.json[l]`.
///
/// Shared by Gemini CLI and Qwen Code, which both store one session per
/// file directly under a per-project `chats/` directory inside `tmp/`.
/// Subagent files nested one level deeper under `chats/<parent>/` are
/// skipped, matching what each tool's own session picker shows.
pub fn list_chat_recording_sessions(
    provider: &str,
    tmp_root: &Path,
    cwd: Option<&Path>,
    parse_meta: impl Fn(&Path) -> Result<AgentSessionMeta>,
) -> Result<Vec<AgentSessionInfo>> {
    collect_recent_sessions(
        provider,
        tmp_root,
        cwd,
        chat_recording_files(tmp_root)?,
        parse_meta,
    )
}

fn collect_recent_sessions(
    provider: &str,
    root: &Path,
    cwd: Option<&Path>,
    files: Vec<PathBuf>,
    parse_meta: impl Fn(&Path) -> Result<AgentSessionMeta>,
) -> Result<Vec<AgentSessionInfo>> {
    let wanted = cwd.map(WorkspaceMatchTarget::new);
    let mut cache = load_listing_cache(provider, root);
    let mut dirty = false;
    let mut sessions = Vec::new();

    for path in files {
        let Some(stamp) = crate::cache::file_stamp(&path) else {
            // Unstampable file (e.g. racing delete): parse without caching.
            match parse_meta(&path) {
                Ok(meta) => push_session(
                    &mut sessions,
                    path,
                    SystemTime::UNIX_EPOCH,
                    meta,
                    wanted.as_ref(),
                ),
                Err(error) => eprintln!(
                    "warning: failed to parse agent session metadata {}: {error:#}",
                    path.display()
                ),
            }
            continue;
        };

        let meta = match cache.entries.get(&path) {
            Some(entry)
                if entry.mtime_secs == stamp.0
                    && entry.mtime_nanos == stamp.1
                    && entry.size == stamp.2 =>
            {
                entry.meta.clone()
            }
            _ => {
                dirty = true;
                match parse_meta(&path) {
                    Ok(meta) => {
                        cache.entries.insert(
                            path.clone(),
                            ListingEntry {
                                mtime_secs: stamp.0,
                                mtime_nanos: stamp.1,
                                size: stamp.2,
                                meta: meta.clone(),
                            },
                        );
                        meta
                    }
                    Err(error) => {
                        eprintln!(
                            "warning: failed to parse agent session metadata {}: {error:#}",
                            path.display()
                        );
                        continue;
                    }
                }
            }
        };

        push_session(
            &mut sessions,
            path,
            SystemTime::UNIX_EPOCH + Duration::new(stamp.0, stamp.1),
            meta,
            wanted.as_ref(),
        );
    }

    if dirty {
        store_listing_cache(provider, root, &cache);
    }

    sessions.sort_by_key(|session| session.modified);
    sessions.reverse();
    Ok(sessions)
}

fn chat_recording_files(tmp_root: &Path) -> Result<Vec<PathBuf>> {
    if !tmp_root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for project in
        fs::read_dir(tmp_root).with_context(|| format!("Failed to read {}", tmp_root.display()))?
    {
        let chats = project?.path().join("chats");
        if !chats.is_dir() {
            continue;
        }
        for entry in
            fs::read_dir(&chats).with_context(|| format!("Failed to read {}", chats.display()))?
        {
            let path = entry?.path();
            let is_chat_file = path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext == "json" || ext == "jsonl");
            if is_chat_file {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn push_session(
    sessions: &mut Vec<AgentSessionInfo>,
    path: PathBuf,
    modified: SystemTime,
    meta: AgentSessionMeta,
    wanted: Option<&WorkspaceMatchTarget>,
) {
    // Shared policy: no cwd metadata → keep; otherwise path or git-remote match.
    if let Some(wanted) = wanted {
        if !workspace_matches_candidates(wanted, meta.cwd_candidates().map(Path::new)) {
            return;
        }
    }

    sessions.push(AgentSessionInfo {
        modified,
        path,
        id: meta.id,
        cwd: meta.cwd,
        title: meta.title,
    });
}

fn load_listing_cache(provider: &str, root: &Path) -> ListingCache {
    let Ok(bytes) = std::fs::read(crate::cache::listing_cache_path(provider, root)) else {
        return ListingCache::default();
    };
    let Ok(cached) = rmp_serde::from_slice::<ListingCache>(&bytes) else {
        return ListingCache::default();
    };
    if cached.version != LISTING_CACHE_VERSION {
        return ListingCache::default();
    }
    cached
}

fn store_listing_cache(provider: &str, root: &Path, cache: &ListingCache) {
    let cache_entry = ListingCache {
        version: LISTING_CACHE_VERSION,
        entries: cache.entries.clone(),
    };
    let mut serializer = rmp_serde::encode::Serializer::new(Vec::new()).with_struct_map();
    if cache_entry.serialize(&mut serializer).is_err() {
        return;
    }
    crate::cache::write_cache_atomic(
        &crate::cache::listing_cache_path(provider, root),
        &serializer.into_inner(),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env_lock()
    }

    fn session_line(id: &str, cwd: &Path) -> String {
        // json! escapes the path (Windows backslashes are not valid JSON).
        format!("{}\n", json!({ "sessionId": id, "cwd": cwd }))
    }

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
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("SIVTR_HOME");
        std::env::set_var("SIVTR_HOME", dir.path().join("data"));
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

        let sessions = list_recent_jsonl_sessions("Claude", &sessions, Some(&target), |path| {
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

        match previous {
            Some(value) => std::env::set_var("SIVTR_HOME", value),
            None => std::env::remove_var("SIVTR_HOME"),
        }
    }

    #[test]
    fn keeps_sessions_without_cwd_when_filtering_by_cwd() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("SIVTR_HOME");
        std::env::set_var("SIVTR_HOME", dir.path().join("data"));
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

        let listed = list_recent_jsonl_sessions("Hermes", &sessions, Some(&target), |path| {
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

        match previous {
            Some(value) => std::env::set_var("SIVTR_HOME", value),
            None => std::env::remove_var("SIVTR_HOME"),
        }
    }

    #[test]
    fn listing_cache_reuses_meta_for_unchanged_files() {
        let _guard = env_lock();
        let temp = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("SIVTR_HOME");
        std::env::set_var("SIVTR_HOME", temp.path());

        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        for index in 0..3 {
            fs::write(
                sessions.join(format!("s{index}.jsonl")),
                session_line(&format!("s{index}"), temp.path()),
            )
            .unwrap();
        }

        let parses = std::cell::Cell::new(0usize);
        let count_meta = |path: &Path| {
            parses.set(parses.get() + 1);
            parse_jsonl_meta(path, "Test", 50, |meta, value| {
                if meta.id.is_none() {
                    meta.id = value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    meta.add_cwd(cwd);
                }
            })
        };

        let first = list_recent_jsonl_sessions("Test", &sessions, None, count_meta).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(parses.get(), 3);

        let second = list_recent_jsonl_sessions("Test", &sessions, None, count_meta).unwrap();
        assert_eq!(second.len(), 3);
        assert_eq!(
            parses.get(),
            3,
            "unchanged files must not be meta-parsed again"
        );

        match previous {
            Some(value) => std::env::set_var("SIVTR_HOME", value),
            None => std::env::remove_var("SIVTR_HOME"),
        }
    }

    #[test]
    fn listing_cache_reparses_changed_files_only() {
        let _guard = env_lock();
        let temp = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("SIVTR_HOME");
        std::env::set_var("SIVTR_HOME", temp.path());

        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let changed = sessions.join("changed.jsonl");
        fs::write(&changed, session_line("one", temp.path())).unwrap();
        let stable = sessions.join("stable.jsonl");
        fs::write(&stable, session_line("stable", temp.path())).unwrap();

        let parses = std::cell::Cell::new(0usize);
        let count_meta = |path: &Path| {
            parses.set(parses.get() + 1);
            parse_jsonl_meta(path, "Test", 50, |meta, value| {
                if meta.id.is_none() {
                    meta.id = value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    meta.add_cwd(cwd);
                }
            })
        };

        list_recent_jsonl_sessions("Test", &sessions, None, count_meta).unwrap();
        assert_eq!(parses.get(), 2);

        // Rewrite only one file, with a different size so the stamp differs.
        fs::write(
            &changed,
            format!(
                "{}\n",
                json!({ "sessionId": "two", "cwd": temp.path(), "extra": true })
            ),
        )
        .unwrap();

        let sessions = list_recent_jsonl_sessions("Test", &sessions, None, count_meta).unwrap();
        assert_eq!(parses.get(), 3, "only the changed file is re-parsed");
        let ids: Vec<_> = sessions
            .iter()
            .filter_map(|session| session.id.clone())
            .collect();
        assert!(ids.iter().any(|id| id == "two"));
        assert!(ids.iter().any(|id| id == "stable"));

        match previous {
            Some(value) => std::env::set_var("SIVTR_HOME", value),
            None => std::env::remove_var("SIVTR_HOME"),
        }
    }
}
