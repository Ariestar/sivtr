//! Lossless import of Claude account exports into Claude Code-compatible JSONL sessions.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::claude::claude_home;

const MANIFEST_SCHEMA_VERSION: u32 = 1;
const IMPORTER_VERSION: &str = "claude-account-export-v1";
const PROJECT_LABEL_MAX_CHARS: usize = 48;
const PROJECT_HASH_CHARS: usize = 16;

#[derive(Debug, Clone)]
pub struct ClaudeExportImportOptions {
    pub source_dir: PathBuf,
    pub cwd: Option<PathBuf>,
    /// Claude project directory. The importer appends `sivtr-imports/<batch-id>`.
    pub destination: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedSourceFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSession {
    pub source_kind: String,
    pub source_conversation_id: String,
    pub branch_leaf_id: Option<String>,
    pub session_id: String,
    pub path: String,
    pub source_message_ids: Vec<String>,
    pub event_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeExportImportReport {
    pub schema_version: u32,
    pub importer_version: String,
    pub batch_id: String,
    pub source_root: String,
    pub target_cwd: String,
    pub destination: String,
    pub dry_run: bool,
    pub already_imported: bool,
    pub conversation_count: usize,
    pub design_chat_count: usize,
    pub source_message_count: usize,
    pub generated_session_count: usize,
    pub generated_event_count: usize,
    pub source_files: Vec<ImportedSourceFile>,
    pub sessions: Vec<ImportedSession>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportManifest {
    schema_version: u32,
    importer_version: String,
    batch_id: String,
    created_at: String,
    source_root: String,
    target_cwd: String,
    source_files: Vec<ImportedSourceFile>,
    sessions: Vec<ImportedSession>,
    conversation_count: usize,
    design_chat_count: usize,
    source_message_count: usize,
    generated_event_count: usize,
}

#[derive(Debug, Clone)]
struct LoadedSourceFile {
    relative_path: PathBuf,
    bytes: Vec<u8>,
    json: Value,
    report: ImportedSourceFile,
}

#[derive(Debug, Clone)]
struct GeneratedSession {
    report: ImportedSession,
    events: Vec<Value>,
}

#[derive(Default)]
struct GenerationStats {
    conversation_count: usize,
    design_chat_count: usize,
    source_message_count: usize,
    missing_parent_refs: usize,
    file_references: usize,
}

#[derive(Debug, Clone)]
struct ConversationMessage {
    id: String,
    parent_id: Option<String>,
    value: Value,
}

pub fn import_claude_export(
    options: &ClaudeExportImportOptions,
) -> Result<ClaudeExportImportReport> {
    let source_root = canonical_directory(&options.source_dir, "Claude export directory")?;
    let target_cwd = match options.cwd.as_deref() {
        Some(cwd) => canonical_directory(cwd, "target cwd")?,
        None => source_root.clone(),
    };
    let destination_project = match options.destination.as_deref() {
        Some(destination) => absolute_path(destination)?,
        None => default_claude_project_dir(&target_cwd)?,
    };

    let source_files = load_source_files(&source_root)?;
    let batch_id = batch_id(&source_files)?;
    let batch_dir = destination_project.join("sivtr-imports").join(&batch_id);
    let target_cwd_text = path_text(&target_cwd, "target cwd")?.to_string();

    let mut stats = GenerationStats::default();
    let sessions = generate_sessions(&source_files, &target_cwd_text, &mut stats)?;
    let source_file_reports = source_files
        .iter()
        .map(|source| source.report.clone())
        .collect::<Vec<_>>();
    let session_reports = sessions
        .iter()
        .map(|session| session.report.clone())
        .collect::<Vec<_>>();
    let generated_event_count = sessions.iter().map(|session| session.events.len()).sum();
    let mut warnings = Vec::new();
    if stats.missing_parent_refs > 0 {
        warnings.push(format!(
            "{} message parent references point outside the export and were treated as branch roots",
            stats.missing_parent_refs
        ));
    }
    if stats.file_references > 0 {
        warnings.push(format!(
            "{} file references contain metadata only; no binary payload was present in the export",
            stats.file_references
        ));
    }

    let mut report = ClaudeExportImportReport {
        schema_version: MANIFEST_SCHEMA_VERSION,
        importer_version: IMPORTER_VERSION.to_string(),
        batch_id: batch_id.clone(),
        source_root: path_text(&source_root, "source root")?.to_string(),
        target_cwd: target_cwd_text,
        destination: path_text(&batch_dir, "import destination")?.to_string(),
        dry_run: options.dry_run,
        already_imported: false,
        conversation_count: stats.conversation_count,
        design_chat_count: stats.design_chat_count,
        source_message_count: stats.source_message_count,
        generated_session_count: sessions.len(),
        generated_event_count,
        source_files: source_file_reports,
        sessions: session_reports,
        warnings,
    };

    if batch_dir.exists() {
        ensure_existing_batch_matches(&batch_dir, &report)?;
        report.already_imported = true;
        return Ok(report);
    }
    if options.dry_run {
        return Ok(report);
    }

    publish_batch(&destination_project, &source_files, &sessions, &report)?;
    Ok(report)
}

pub fn default_claude_project_dir(cwd: &Path) -> Result<PathBuf> {
    let cwd_text = path_text(cwd, "target cwd")?;
    let path_hash = sha256_hex(cwd_text.as_bytes());
    let label = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .map(safe_project_label)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".to_string());
    let project = format!("{label}-{}", &path_hash[..PROJECT_HASH_CHARS]);
    Ok(claude_home().join("projects").join(project))
}

fn safe_project_label(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .take(PROJECT_LABEL_MAX_CHARS)
        .collect::<String>();
    sanitized
        .trim_matches(|ch| ch == '-' || ch == '_')
        .to_string()
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(path)
        .with_context(|| format!("Failed to resolve {label}: {}", path.display()))?;
    let path = without_windows_verbatim_prefix(path);
    if !path.is_dir() {
        bail!("{label} is not a directory: {}", path.display());
    }
    Ok(path)
}

#[cfg(windows)]
fn without_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn without_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("Failed to resolve current directory")?
        .join(path))
}

fn path_text<'a>(path: &'a Path, label: &str) -> Result<&'a str> {
    path.to_str()
        .with_context(|| format!("{label} is not valid Unicode: {}", path.display()))
}

fn load_source_files(source_root: &Path) -> Result<Vec<LoadedSourceFile>> {
    let mut relative_paths = Vec::new();
    for name in ["conversations.json", "memories.json", "users.json"] {
        let relative = PathBuf::from(name);
        if source_root.join(&relative).is_file() {
            relative_paths.push(relative);
        }
    }

    let design_dir = source_root.join("design_chats");
    if design_dir.is_dir() {
        for entry in fs::read_dir(&design_dir)
            .with_context(|| format!("Failed to read {}", design_dir.display()))?
        {
            let entry = entry.context("Failed to read design_chats entry")?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                let name = path
                    .file_name()
                    .context("design chat path has no file name")?;
                relative_paths.push(PathBuf::from("design_chats").join(name));
            }
        }
    }
    relative_paths.sort();

    if !relative_paths
        .iter()
        .any(|path| path == Path::new("conversations.json"))
        && !relative_paths
            .iter()
            .any(|path| path.starts_with("design_chats"))
    {
        bail!(
            "No supported Claude conversations were found in {}",
            source_root.display()
        );
    }

    relative_paths
        .into_iter()
        .map(|relative_path| load_source_file(source_root, relative_path))
        .collect()
}

fn load_source_file(source_root: &Path, relative_path: PathBuf) -> Result<LoadedSourceFile> {
    let path = source_root.join(&relative_path);
    let bytes = fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .with_context(|| format!("{} is not strict UTF-8", path.display()))?;
    let json_text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let json = serde_json::from_str(json_text)
        .with_context(|| format!("Failed to parse {} as JSON", path.display()))?;
    let relative_text = path_text(&relative_path, "source relative path")?.replace('\\', "/");
    let report = ImportedSourceFile {
        path: relative_text,
        size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    };
    Ok(LoadedSourceFile {
        relative_path,
        bytes,
        json,
        report,
    })
}

fn batch_id(source_files: &[LoadedSourceFile]) -> Result<String> {
    let mut hasher = Sha256::new();
    for source in source_files {
        let path = source.report.path.as_bytes();
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path);
        hasher.update((source.bytes.len() as u64).to_le_bytes());
        hasher.update(&source.bytes);
    }
    Ok(hex(&hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn generate_sessions(
    source_files: &[LoadedSourceFile],
    cwd: &str,
    stats: &mut GenerationStats,
) -> Result<Vec<GeneratedSession>> {
    let mut sessions = Vec::new();
    let mut conversation_ids = HashSet::new();
    let mut message_ids = HashSet::new();

    for source in source_files {
        if source.relative_path == Path::new("conversations.json") {
            generate_conversations(
                &source.json,
                cwd,
                stats,
                &mut conversation_ids,
                &mut message_ids,
                &mut sessions,
            )?;
        } else if source.relative_path.starts_with("design_chats") {
            generate_design_chat(
                &source.json,
                &source.report.path,
                cwd,
                stats,
                &mut conversation_ids,
                &mut message_ids,
                &mut sessions,
            )?;
        }
    }
    Ok(sessions)
}

fn generate_conversations(
    root: &Value,
    cwd: &str,
    stats: &mut GenerationStats,
    conversation_ids: &mut HashSet<String>,
    message_ids: &mut HashSet<String>,
    sessions: &mut Vec<GeneratedSession>,
) -> Result<()> {
    let conversations = root
        .as_array()
        .context("conversations.json root must be an array")?;
    for conversation in conversations {
        let id = required_string(conversation, "uuid", "conversation")?;
        if !conversation_ids.insert(id.to_string()) {
            bail!("Duplicate conversation UUID `{id}`");
        }
        let messages = conversation
            .get("chat_messages")
            .and_then(Value::as_array)
            .with_context(|| format!("Conversation `{id}` has no chat_messages array"))?;
        stats.conversation_count += 1;
        stats.source_message_count += messages.len();

        let mut parsed = Vec::with_capacity(messages.len());
        for message in messages {
            let message_id = required_string(message, "uuid", "conversation message")?;
            if !message_ids.insert(message_id.to_string()) {
                bail!("Duplicate message UUID `{message_id}`");
            }
            validate_web_sender(message, message_id)?;
            stats.file_references += message
                .get("files")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            parsed.push(ConversationMessage {
                id: message_id.to_string(),
                parent_id: optional_string(message, "parent_message_uuid")?,
                value: message.clone(),
            });
        }

        let branches = conversation_branches(&parsed, &mut stats.missing_parent_refs)?;
        let metadata = conversation_metadata(conversation, "chat_messages")?;
        let title = conversation
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| conversation.get("summary").and_then(Value::as_str))
            .unwrap_or(id);

        if branches.is_empty() {
            let event = metadata_event(id, cwd, title, "conversation", metadata, conversation)?;
            sessions.push(generated_session(
                "conversation",
                id,
                None,
                id,
                Vec::new(),
                vec![event],
            ));
            continue;
        }

        let split = branches.len() > 1;
        for branch in branches {
            let leaf_id = branch
                .last()
                .context("conversation branch unexpectedly has no leaf")?
                .id
                .clone();
            let session_id = if split {
                deterministic_session_id(id, &leaf_id)
            } else {
                id.to_string()
            };
            let mut events = Vec::with_capacity(branch.len());
            let mut branch_message_ids = Vec::with_capacity(branch.len());
            for message in branch {
                events.push(web_message_event(
                    &session_id,
                    cwd,
                    title,
                    id,
                    &leaf_id,
                    &metadata,
                    message,
                )?);
                branch_message_ids.push(message.id.clone());
            }
            sessions.push(generated_session(
                "conversation",
                id,
                split.then_some(leaf_id),
                &session_id,
                branch_message_ids,
                events,
            ));
        }
    }
    Ok(())
}

fn generate_design_chat(
    conversation: &Value,
    source_path: &str,
    cwd: &str,
    stats: &mut GenerationStats,
    conversation_ids: &mut HashSet<String>,
    message_ids: &mut HashSet<String>,
    sessions: &mut Vec<GeneratedSession>,
) -> Result<()> {
    let id = required_string(conversation, "uuid", "design chat")?;
    if !conversation_ids.insert(id.to_string()) {
        bail!("Duplicate conversation UUID `{id}`");
    }
    let messages = conversation
        .get("messages")
        .and_then(Value::as_array)
        .with_context(|| format!("Design chat `{id}` has no messages array"))?;
    stats.design_chat_count += 1;
    stats.source_message_count += messages.len();
    let title = conversation
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(id);
    let metadata = conversation_metadata(conversation, "messages")?;
    let mut events = Vec::with_capacity(messages.len().max(1));
    let mut source_message_ids = Vec::with_capacity(messages.len());

    for message in messages {
        let message_id = required_string(message, "uuid", "design chat message")?;
        if !message_ids.insert(message_id.to_string()) {
            bail!("Duplicate message UUID `{message_id}`");
        }
        let outer_role = required_string(message, "role", "design chat message")?;
        let inner = message
            .get("content")
            .and_then(Value::as_object)
            .with_context(|| format!("Design chat message `{message_id}` has no content object"))?;
        let inner_role = inner
            .get("role")
            .and_then(Value::as_str)
            .with_context(|| format!("Design chat message `{message_id}` has no inner role"))?;
        if outer_role != inner_role {
            bail!(
                "Design chat message `{message_id}` role mismatch: outer `{outer_role}`, inner `{inner_role}`"
            );
        }
        let event_type = normalized_role(outer_role, message_id)?;
        let content = inner
            .get("content")
            .cloned()
            .with_context(|| format!("Design chat message `{message_id}` has no inner content"))?;
        if !content.is_string() {
            bail!("Design chat message `{message_id}` content must be a string");
        }
        let timestamp = required_string(message, "created_at", "design chat message")?;
        events.push(json!({
            "type": event_type,
            "uuid": message_id,
            "sessionId": id,
            "cwd": cwd,
            "timestamp": timestamp,
            "customTitle": title,
            "message": {
                "role": event_type,
                "content": content,
            },
            "sivtrImport": {
                "format": "claude-design-chat-export",
                "sourcePath": source_path,
                "sourceConversationId": id,
                "sourceMessageId": message_id,
                "sourceRole": outer_role,
                "conversation": metadata,
                "originalMessage": message,
            }
        }));
        source_message_ids.push(message_id.to_string());
    }

    if events.is_empty() {
        events.push(metadata_event(
            id,
            cwd,
            title,
            "design_chat",
            metadata,
            conversation,
        )?);
    }
    sessions.push(generated_session(
        "design_chat",
        id,
        None,
        id,
        source_message_ids,
        events,
    ));
    Ok(())
}

fn validate_web_sender(message: &Value, message_id: &str) -> Result<()> {
    let sender = required_string(message, "sender", "conversation message")?;
    match sender {
        "human" | "assistant" => Ok(()),
        _ => bail!("Unknown sender `{sender}` in message `{message_id}`"),
    }
}

fn normalized_role<'a>(role: &'a str, message_id: &str) -> Result<&'a str> {
    match role {
        "human" | "user" => Ok("user"),
        "assistant" => Ok("assistant"),
        _ => bail!("Unknown role `{role}` in message `{message_id}`"),
    }
}

fn conversation_branches<'a>(
    messages: &'a [ConversationMessage],
    missing_parent_refs: &mut usize,
) -> Result<Vec<Vec<&'a ConversationMessage>>> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let by_id = messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id.as_str(), index))
        .collect::<HashMap<_, _>>();

    for message in messages {
        let mut current = message;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current.id.as_str()) {
                bail!("Cycle detected at message `{}`", current.id);
            }
            let Some(parent_id) = current.parent_id.as_deref() else {
                break;
            };
            let Some(parent_index) = by_id.get(parent_id).copied() else {
                break;
            };
            current = &messages[parent_index];
        }
    }

    let mut known_parents = HashSet::new();
    for message in messages {
        if let Some(parent_id) = message.parent_id.as_deref() {
            if by_id.contains_key(parent_id) {
                known_parents.insert(parent_id);
            } else {
                *missing_parent_refs += 1;
            }
        }
    }
    let leaves = messages
        .iter()
        .filter(|message| !known_parents.contains(message.id.as_str()))
        .collect::<Vec<_>>();
    if leaves.is_empty() {
        bail!("Conversation graph has no leaf messages");
    }

    let mut branches = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let mut branch = Vec::new();
        let mut current = leaf;
        loop {
            branch.push(current);
            let Some(parent_id) = current.parent_id.as_deref() else {
                break;
            };
            let Some(parent_index) = by_id.get(parent_id).copied() else {
                break;
            };
            current = &messages[parent_index];
        }
        branch.reverse();
        branches.push(branch);
    }
    Ok(branches)
}

fn web_message_event(
    session_id: &str,
    cwd: &str,
    title: &str,
    conversation_id: &str,
    branch_leaf_id: &str,
    conversation_metadata: &Value,
    message: &ConversationMessage,
) -> Result<Value> {
    let sender = required_string(&message.value, "sender", "conversation message")?;
    let event_type = normalized_role(sender, &message.id)?;
    let timestamp = required_string(&message.value, "created_at", "conversation message")?;
    let content = message
        .value
        .get("content")
        .cloned()
        .context("conversation message has no content field")?;
    if !content.is_array() && !content.is_string() {
        bail!(
            "Message `{}` content must be an array or string",
            message.id
        );
    }

    Ok(json!({
        "type": event_type,
        "uuid": message.id,
        "parentUuid": message.parent_id,
        "sessionId": session_id,
        "cwd": cwd,
        "timestamp": timestamp,
        "customTitle": title,
        "message": {
            "role": event_type,
            "content": content,
        },
        "sivtrImport": {
            "format": "claude-account-export",
            "sourceConversationId": conversation_id,
            "sourceMessageId": message.id,
            "sourceParentMessageId": message.parent_id,
            "sourceSender": sender,
            "branchLeafId": branch_leaf_id,
            "conversation": conversation_metadata,
            "originalMessage": message.value,
        }
    }))
}

fn metadata_event(
    session_id: &str,
    cwd: &str,
    title: &str,
    source_kind: &str,
    metadata: Value,
    original_conversation: &Value,
) -> Result<Value> {
    let timestamp = original_conversation
        .get("created_at")
        .and_then(Value::as_str)
        .or_else(|| {
            original_conversation
                .get("updated_at")
                .and_then(Value::as_str)
        });
    Ok(json!({
        "type": "sivtr-import-metadata",
        "sessionId": session_id,
        "cwd": cwd,
        "timestamp": timestamp,
        "customTitle": title,
        "sivtrImport": {
            "format": source_kind,
            "sourceConversationId": session_id,
            "conversation": metadata,
            "originalConversation": original_conversation,
        }
    }))
}

fn conversation_metadata(conversation: &Value, messages_key: &str) -> Result<Value> {
    let mut metadata = conversation
        .as_object()
        .with_context(|| format!("conversation must be an object containing `{messages_key}`"))?
        .clone();
    metadata.remove(messages_key);
    Ok(Value::Object(metadata))
}

fn generated_session(
    source_kind: &str,
    source_conversation_id: &str,
    branch_leaf_id: Option<String>,
    session_id: &str,
    source_message_ids: Vec<String>,
    events: Vec<Value>,
) -> GeneratedSession {
    let path = format!("sessions/{session_id}.jsonl");
    GeneratedSession {
        report: ImportedSession {
            source_kind: source_kind.to_string(),
            source_conversation_id: source_conversation_id.to_string(),
            branch_leaf_id,
            session_id: session_id.to_string(),
            path,
            source_message_ids,
            event_count: events.len(),
        },
        events,
    }
}

fn deterministic_session_id(conversation_id: &str, leaf_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(conversation_id.as_bytes());
    hash.update([0]);
    hash.update(leaf_id.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn required_string<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("{label} has no string `{key}`"))
}

fn optional_string(value: &Value, key: &str) -> Result<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => bail!("`{key}` must be a string or null"),
    }
}

fn ensure_existing_batch_matches(
    batch_dir: &Path,
    report: &ClaudeExportImportReport,
) -> Result<()> {
    let manifest_path = batch_dir.join("manifest.json");
    let bytes = fs::read(&manifest_path).with_context(|| {
        format!(
            "Import destination exists without a readable manifest: {}",
            manifest_path.display()
        )
    })?;
    let manifest: ImportManifest = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "Import destination manifest is invalid: {}",
            manifest_path.display()
        )
    })?;
    if manifest.batch_id != report.batch_id
        || manifest.source_files != report.source_files
        || manifest.source_root != report.source_root
        || manifest.target_cwd != report.target_cwd
    {
        bail!(
            "Import destination conflicts with this batch and will not be overwritten: {}",
            batch_dir.display()
        );
    }
    Ok(())
}

fn publish_batch(
    destination_project: &Path,
    source_files: &[LoadedSourceFile],
    sessions: &[GeneratedSession],
    report: &ClaudeExportImportReport,
) -> Result<()> {
    let imports_root = destination_project.join("sivtr-imports");
    fs::create_dir_all(&imports_root).with_context(|| {
        format!(
            "Failed to create Claude import directory {}",
            imports_root.display()
        )
    })?;
    set_private_dir_permissions(&imports_root)?;

    let temp_dir = imports_root.join(format!(
        ".tmp-{}-{}-{}",
        report.batch_id,
        std::process::id(),
        Uuid::new_v4()
    ));
    let batch_dir = imports_root.join(&report.batch_id);
    let result = write_batch_contents(&temp_dir, source_files, sessions, report).and_then(|()| {
        if batch_dir.exists() {
            bail!(
                "Import destination appeared during publish and will not be overwritten: {}",
                batch_dir.display()
            );
        }
        fs::rename(&temp_dir, &batch_dir).with_context(|| {
            format!(
                "Failed to publish Claude import batch {}",
                batch_dir.display()
            )
        })?;
        Ok(())
    });
    if result.is_err() && temp_dir.parent() == Some(imports_root.as_path()) {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    result
}

fn write_batch_contents(
    temp_dir: &Path,
    source_files: &[LoadedSourceFile],
    sessions: &[GeneratedSession],
    report: &ClaudeExportImportReport,
) -> Result<()> {
    let source_dir = temp_dir.join("source");
    let sessions_dir = temp_dir.join("sessions");
    fs::create_dir_all(temp_dir)
        .with_context(|| format!("Failed to create {}", temp_dir.display()))?;
    set_private_dir_permissions(temp_dir)?;
    fs::create_dir_all(&source_dir)
        .with_context(|| format!("Failed to create {}", source_dir.display()))?;
    set_private_dir_permissions(&source_dir)?;
    fs::create_dir_all(&sessions_dir)
        .with_context(|| format!("Failed to create {}", sessions_dir.display()))?;
    set_private_dir_permissions(&sessions_dir)?;

    for source in source_files {
        let destination = source_dir.join(&source.relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
            set_private_dir_permissions(parent)?;
        }
        write_bytes(&destination, &source.bytes)?;
        let copied = fs::read(&destination)
            .with_context(|| format!("Failed to verify {}", destination.display()))?;
        if sha256_hex(&copied) != source.report.sha256 {
            bail!("Snapshot hash mismatch for {}", destination.display());
        }
    }

    for session in sessions {
        let destination = temp_dir.join(&session.report.path);
        write_jsonl(&destination, &session.events)?;
    }

    let manifest = ImportManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        importer_version: IMPORTER_VERSION.to_string(),
        batch_id: report.batch_id.clone(),
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        source_root: report.source_root.clone(),
        target_cwd: report.target_cwd.clone(),
        source_files: report.source_files.clone(),
        sessions: report.sessions.clone(),
        conversation_count: report.conversation_count,
        design_chat_count: report.design_chat_count,
        source_message_count: report.source_message_count,
        generated_event_count: report.generated_event_count,
    };
    let bytes =
        serde_json::to_vec_pretty(&manifest).context("Failed to serialize import manifest")?;
    write_bytes(&temp_dir.join("manifest.json"), &bytes)
}

fn write_jsonl(path: &Path, events: &[Value]) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("Failed to create {}", path.display()))?;
    set_private_file_permissions(path)?;
    let mut writer = BufWriter::new(file);
    for event in events {
        serde_json::to_writer(&mut writer, event)
            .with_context(|| format!("Failed to serialize event for {}", path.display()))?;
        writer
            .write_all(b"\n")
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    writer
        .flush()
        .with_context(|| format!("Failed to flush {}", path.display()))?;
    writer
        .get_ref()
        .sync_all()
        .with_context(|| format!("Failed to sync {}", path.display()))
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        File::create(path).with_context(|| format!("Failed to create {}", path.display()))?;
    set_private_file_permissions(path)?;
    file.write_all(bytes)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("Failed to sync {}", path.display()))
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("Failed to secure directory {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("Failed to secure file {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde_json::{json, Value};

    use super::{import_claude_export, ClaudeExportImportOptions};
    use crate::ai::{AgentBlockKind, AgentSessionProvider};
    use crate::claude::ClaudeProvider;

    const USER_TEXT: &str = "中文 한글 emoji 😀 e\u{301}\u{200b}\r\nquote \" slash \\ nul \0";

    fn write_json(path: &Path, value: &Value) {
        let bytes = serde_json::to_vec(value).expect("serialize fixture");
        fs::write(path, bytes).expect("write fixture");
    }

    fn fixture(root: &Path) {
        let long_text = format!("long-start\r\n{}\nlong-end", "长😀".repeat(32_768));
        fs::create_dir_all(root.join("design_chats")).expect("design dir");
        write_json(
            &root.join("conversations.json"),
            &json!([{
                "uuid": "11111111-1111-4111-8111-111111111111",
                "name": "原始标题",
                "summary": "summary",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:05:00Z",
                "account": {"uuid": "account-1"},
                "chat_messages": [
                    {
                        "uuid": "m1",
                        "text": USER_TEXT,
                        "content": [{"type": "text", "text": USER_TEXT}],
                        "sender": "human",
                        "created_at": "2026-01-01T00:00:00Z",
                        "updated_at": "2026-01-01T00:00:00Z",
                        "attachments": [{"file_name": "资料.txt", "file_size": 4, "file_type": "text/plain", "extracted_content": USER_TEXT}],
                        "files": [{"file_name": "missing.bin", "file_uuid": "file-1"}],
                        "parent_message_uuid": "outside-export"
                    },
                    {
                        "uuid": "m2",
                        "text": "assistant answer",
                        "content": [
                            {"type": "thinking", "thinking": "preserved but hidden"},
                            {"type": "text", "text": "assistant answer"},
                            {"type": "tool_use", "id": "tool-1", "name": "lookup", "input": {"q": USER_TEXT}}
                        ],
                        "sender": "assistant",
                        "created_at": "2026-01-01T00:01:00Z",
                        "updated_at": "2026-01-01T00:01:00Z",
                        "attachments": [],
                        "files": [],
                        "parent_message_uuid": "m1"
                    },
                    {
                        "uuid": "m3",
                        "text": "next",
                        "content": [
                            {"type": "tool_result", "tool_use_id": "tool-1", "content": "tool result"},
                            {"type": "text", "text": "next"}
                        ],
                        "sender": "human",
                        "created_at": "2026-01-01T00:02:00Z",
                        "updated_at": "2026-01-01T00:02:00Z",
                        "attachments": [],
                        "files": [],
                        "parent_message_uuid": "m2"
                    },
                    {
                        "uuid": "m4",
                        "text": "top-level text must not replace empty content",
                        "content": [],
                        "sender": "assistant",
                        "created_at": "2026-01-01T00:03:00Z",
                        "updated_at": "2026-01-01T00:03:00Z",
                        "attachments": [],
                        "files": [],
                        "parent_message_uuid": "m3"
                    },
                    {
                        "uuid": "m5",
                        "text": long_text,
                        "content": [{"type": "text", "text": long_text}],
                        "sender": "assistant",
                        "created_at": "2026-01-01T00:04:00Z",
                        "updated_at": "2026-01-01T00:04:00Z",
                        "attachments": [],
                        "files": [],
                        "parent_message_uuid": "m3"
                    }
                ]
            }]),
        );
        write_json(
            &root.join("design_chats").join("design-1.json"),
            &json!({
                "uuid": "22222222-2222-4222-8222-222222222222",
                "title": "设计对话",
                "project": {"uuid": "project-1"},
                "created_at": "2026-01-02T00:00:00Z",
                "updated_at": "2026-01-02T00:01:00Z",
                "messages": [
                    {
                        "uuid": "d1",
                        "role": "user",
                        "created_at": "2026-01-02T00:00:00Z",
                        "content": {"id": "d1", "role": "user", "content": USER_TEXT, "timestamp": "2026-01-02T00:00:00Z"}
                    },
                    {
                        "uuid": "d2",
                        "role": "assistant",
                        "created_at": "2026-01-02T00:01:00Z",
                        "content": {"id": "d2", "role": "assistant", "content": "设计回复", "timestamp": "2026-01-02T00:01:00Z"}
                    }
                ]
            }),
        );
        write_json(
            &root.join("design_chats").join("design-empty.json"),
            &json!({
                "uuid": "33333333-3333-4333-8333-333333333333",
                "title": "空对话",
                "project": {},
                "created_at": "2026-01-03T00:00:00Z",
                "updated_at": "2026-01-03T00:00:00Z",
                "messages": []
            }),
        );
        write_json(
            &root.join("memories.json"),
            &json!([{"account_uuid": "account-1", "conversations_memory": USER_TEXT}]),
        );
        write_json(
            &root.join("users.json"),
            &json!([{"uuid": "account-1", "full_name": "测试用户"}]),
        );
    }

    fn options(source: &Path, destination: &Path, dry_run: bool) -> ClaudeExportImportOptions {
        ClaudeExportImportOptions {
            source_dir: source.to_path_buf(),
            cwd: Some(source.to_path_buf()),
            destination: Some(destination.to_path_buf()),
            dry_run,
        }
    }

    #[cfg(windows)]
    #[test]
    fn canonical_directory_does_not_leak_windows_verbatim_prefix() {
        let temp = tempfile::tempdir().expect("temp dir");

        let canonical =
            super::canonical_directory(temp.path(), "test directory").expect("canonical directory");
        let canonical_text = canonical.to_str().expect("Unicode test path");
        let project = super::default_claude_project_dir(&canonical).expect("Claude project dir");
        let project_name = project
            .file_name()
            .and_then(|name| name.to_str())
            .expect("Unicode project name");

        assert!(!canonical_text.starts_with(r"\\?\"));
        assert!(!project_name.starts_with("----"));
    }

    #[test]
    fn default_project_directory_is_bounded_and_path_unique() {
        let first = super::default_claude_project_dir(Path::new("/foo/bar"))
            .expect("first project directory");
        let second = super::default_claude_project_dir(Path::new("/foo-bar"))
            .expect("second project directory");
        let long = super::default_claude_project_dir(&PathBuf::from("x".repeat(400)))
            .expect("long project directory");

        assert_ne!(first.file_name(), second.file_name());
        assert!(first
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("bar-") && name.len() <= 65));
        assert!(long
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.len() <= 65));
    }

    #[test]
    fn imports_losslessly_with_roles_branches_and_idempotency() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("export");
        let destination = temp.path().join("claude-project");
        fs::create_dir_all(&source).expect("source dir");
        fixture(&source);

        let dry = import_claude_export(&options(&source, &destination, true)).expect("dry run");
        assert!(dry.dry_run);
        assert!(!destination.exists());
        assert_eq!(dry.conversation_count, 1);
        assert_eq!(dry.design_chat_count, 2);
        assert_eq!(dry.source_message_count, 7);
        assert_eq!(dry.generated_session_count, 4);
        assert_eq!(dry.generated_event_count, 11);

        let report = import_claude_export(&options(&source, &destination, false)).expect("import");
        assert!(!report.already_imported);
        let batch = PathBuf::from(&report.destination);
        assert!(batch.join("manifest.json").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for directory in [&batch, &batch.join("source"), &batch.join("sessions")] {
                let mode = fs::metadata(directory)
                    .expect("import directory metadata")
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o700, "{} should be private", directory.display());
            }
        }
        assert_eq!(
            fs::read(batch.join("source").join("conversations.json")).expect("snapshot"),
            fs::read(source.join("conversations.json")).expect("source")
        );

        let branch = report
            .sessions
            .iter()
            .find(|session| session.branch_leaf_id.as_deref() == Some("m4"))
            .expect("m4 branch");
        let path = batch.join(&branch.path);
        let bytes = fs::read(&path).expect("session bytes");
        assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let text = std::str::from_utf8(&bytes).expect("strict utf-8");
        let events = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("event json"))
            .collect::<Vec<_>>();
        assert_eq!(events[0]["message"]["role"], "user");
        assert_eq!(events[1]["message"]["role"], "assistant");
        assert_eq!(
            events[0]["sivtrImport"]["originalMessage"]["text"],
            USER_TEXT
        );
        assert_eq!(
            events[0]["sivtrImport"]["originalMessage"]["attachments"][0]["extracted_content"],
            USER_TEXT
        );
        assert_eq!(
            events[0]["sivtrImport"]["originalMessage"]["files"][0]["file_uuid"],
            "file-1"
        );
        assert_eq!(events[0]["message"]["content"][0]["text"], USER_TEXT);
        assert_eq!(events[1]["message"]["content"][0]["type"], "thinking");
        assert_eq!(events[3]["message"]["content"], json!([]));
        assert_eq!(
            events[3]["sivtrImport"]["originalMessage"]["text"],
            "top-level text must not replace empty content"
        );

        let session = ClaudeProvider
            .parse_session_file(&path)
            .expect("parse imported session");
        assert!(session
            .blocks
            .iter()
            .any(|block| block.kind == AgentBlockKind::User && block.text == USER_TEXT));
        assert!(session.blocks.iter().any(|block| {
            block.kind == AgentBlockKind::Assistant && block.text == "assistant answer"
        }));
        assert!(!session
            .blocks
            .iter()
            .any(|block| block.text.contains("preserved but hidden")));
        assert!(!session.blocks.iter().any(|block| matches!(
            block.kind,
            AgentBlockKind::ToolCall | AgentBlockKind::ToolOutput
        )));

        let repeated =
            import_claude_export(&options(&source, &destination, false)).expect("repeat import");
        assert!(repeated.already_imported);
        assert_eq!(repeated.batch_id, report.batch_id);

        let long_branch = report
            .sessions
            .iter()
            .find(|session| session.branch_leaf_id.as_deref() == Some("m5"))
            .expect("m5 branch");
        let long_events = fs::read_to_string(batch.join(&long_branch.path))
            .expect("read long branch")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("long event json"))
            .collect::<Vec<_>>();
        assert_eq!(
            long_events.last().expect("m5 event")["message"]["content"][0]["text"],
            format!("long-start\r\n{}\nlong-end", "长😀".repeat(32_768))
        );
    }

    #[test]
    fn publish_failure_removes_temporary_batch() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("export");
        let destination = temp.path().join("claude-project");
        fs::create_dir_all(&source).expect("source dir");
        fixture(&source);

        let report = import_claude_export(&options(&source, &destination, true)).expect("dry run");
        let source_files = super::load_source_files(&source).expect("load source files");
        let invalid_session = super::GeneratedSession {
            report: super::ImportedSession {
                source_kind: "conversation".to_string(),
                source_conversation_id: "broken".to_string(),
                branch_leaf_id: None,
                session_id: "broken".to_string(),
                path: "sessions".to_string(),
                source_message_ids: Vec::new(),
                event_count: 1,
            },
            events: vec![json!({"type": "user"})],
        };

        super::publish_batch(&destination, &source_files, &[invalid_session], &report)
            .expect_err("session path should collide with sessions directory");

        let imports_root = destination.join("sivtr-imports");
        assert!(!imports_root.join(&report.batch_id).exists());
        assert_eq!(
            fs::read_dir(&imports_root).expect("imports root").count(),
            0
        );
    }

    #[test]
    fn rejects_invalid_utf8_without_writing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("export");
        let destination = temp.path().join("claude-project");
        fs::create_dir_all(&source).expect("source dir");
        fs::write(source.join("conversations.json"), [0xff, 0xfe]).expect("invalid fixture");

        let error = import_claude_export(&options(&source, &destination, false))
            .expect_err("invalid utf-8 should fail");

        assert!(format!("{error:#}").contains("not strict UTF-8"));
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_unknown_sender_duplicate_ids_and_cycles() {
        for (label, messages, expected) in [
            (
                "sender",
                json!([{
                    "uuid": "m1", "sender": "system", "created_at": "2026-01-01T00:00:00Z",
                    "content": [], "parent_message_uuid": null, "files": []
                }]),
                "Unknown sender",
            ),
            (
                "duplicate",
                json!([
                    {"uuid": "m1", "sender": "human", "created_at": "2026-01-01T00:00:00Z", "content": [], "parent_message_uuid": null, "files": []},
                    {"uuid": "m1", "sender": "assistant", "created_at": "2026-01-01T00:01:00Z", "content": [], "parent_message_uuid": "m1", "files": []}
                ]),
                "Duplicate message UUID",
            ),
            (
                "cycle",
                json!([
                    {"uuid": "m1", "sender": "human", "created_at": "2026-01-01T00:00:00Z", "content": [], "parent_message_uuid": "m2", "files": []},
                    {"uuid": "m2", "sender": "assistant", "created_at": "2026-01-01T00:01:00Z", "content": [], "parent_message_uuid": "m1", "files": []}
                ]),
                "Cycle detected",
            ),
        ] {
            let temp = tempfile::tempdir().expect("temp dir");
            let source = temp.path().join(label);
            fs::create_dir_all(&source).expect("source dir");
            write_json(
                &source.join("conversations.json"),
                &json!([{"uuid": label, "chat_messages": messages}]),
            );
            let error =
                import_claude_export(&options(&source, &temp.path().join("destination"), false))
                    .expect_err("fixture should fail");
            assert!(format!("{error:#}").contains(expected));
        }
    }

    #[test]
    fn rejects_design_role_mismatch_and_destination_conflict() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("export");
        let destination = temp.path().join("claude-project");
        fs::create_dir_all(source.join("design_chats")).expect("design dir");
        write_json(
            &source.join("design_chats").join("bad.json"),
            &json!({
                "uuid": "design-bad",
                "messages": [{
                    "uuid": "d1", "role": "user", "created_at": "2026-01-01T00:00:00Z",
                    "content": {"role": "assistant", "content": "text"}
                }]
            }),
        );
        let error = import_claude_export(&options(&source, &destination, false))
            .expect_err("role mismatch");
        assert!(format!("{error:#}").contains("role mismatch"));
        assert!(!destination.exists());

        fs::remove_dir_all(&source).expect("replace fixture");
        fs::create_dir_all(&source).expect("source dir");
        fixture(&source);
        let dry = import_claude_export(&options(&source, &destination, true)).expect("dry run");
        let batch = PathBuf::from(&dry.destination);
        fs::create_dir_all(&batch).expect("conflicting batch");
        fs::write(batch.join("manifest.json"), b"{}").expect("bad manifest");

        let error = import_claude_export(&options(&source, &destination, false))
            .expect_err("conflict should fail");
        assert!(format!("{error:#}").contains("manifest is invalid"));
        assert_eq!(
            fs::read(batch.join("manifest.json")).expect("manifest"),
            b"{}"
        );
    }
}
