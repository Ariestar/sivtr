//! Unified cloud-backed session history: catalog, encrypted objects, and capabilities.
//!
//! The archive database is never uploaded. Each session is exported as
//! canonical JSONL, split into chunks, compressed, encrypted, and stored by
//! plaintext content hash. The encrypted manifest is written last so a crash
//! during sync leaves the previous generation readable.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{ensure, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::archive::store::{self, BlobMode, SessionUpsert};
use crate::record::WorkRecord;
use crate::workspace;

pub const SCHEMA_VERSION: u32 = 1;
pub const MASTER_KEY_BYTES: usize = 32;
pub const DEFAULT_CHUNK_BYTES: usize = 1024 * 1024;

const CATALOG_KEY: &str = "catalog.json.enc";
const ENVELOPE_MAGIC: &[u8; 8] = b"SIVBKP01";
const ENVELOPE_HEADER_BYTES: usize = 8 + 1 + 12;
const TAG_BYTES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub hash: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub provider: String,
    pub session_id: String,
    pub source_path: String,
    pub cwd: Option<String>,
    pub workspace_key: String,
    pub title: Option<String>,
    pub project: String,
    pub starred: bool,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub content_hash: String,
    pub record_count: usize,
    pub preview: String,
    pub chunks: Vec<ChunkRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub device_id: String,
    pub generation: u64,
    pub created_at: String,
    pub sessions: Vec<SessionEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPushReport {
    pub generation: u64,
    pub sessions_total: usize,
    pub sessions_written: usize,
    pub chunks_uploaded: usize,
    pub chunks_reused: usize,
    pub bytes_uploaded: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPullReport {
    pub generation: u64,
    pub sessions_added: usize,
    pub sessions_unchanged: usize,
    pub records_added: usize,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub generation: u64,
    pub sessions: usize,
    pub chunks_checked: usize,
    pub bytes_verified: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub generation: u64,
    pub created_at: String,
    pub device_id: String,
    pub local_sessions: usize,
    pub remote_sessions: usize,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSettings {
    pub remote: String,
    pub key_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityScope {
    All,
    Session(String),
    Provider(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub schema_version: u32,
    pub id: String,
    pub scope: CapabilityScope,
    pub wrapped_payload: Vec<u8>,
    pub ephemeral_public: [u8; 32],
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopedSession {
    pub entry: SessionEntry,
    pub session_key: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityPayload {
    pub sessions: Vec<ScopedSession>,
}

/// Blocking object backend. Implementations must keep secrets out of errors.
pub trait CloudBackend {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<bool>;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub fn settings_path() -> PathBuf {
    workspace::home_dir().join("cloud.toml")
}

pub fn load_settings() -> Result<Option<CloudSettings>> {
    let path = settings_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read cloud settings {}", path.display()))?;
    let settings: CloudSettings = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse cloud settings {}", path.display()))?;
    Ok(Some(settings))
}

pub fn save_settings(settings: &CloudSettings) -> Result<()> {
    let path = settings_path();
    let contents =
        toml::to_string_pretty(settings).context("Failed to serialize cloud settings")?;
    fs::write(&path, contents)
        .with_context(|| format!("Failed to write cloud settings {}", path.display()))
}

pub fn create_library_key(path: &Path) -> Result<[u8; 32]> {
    ensure!(
        !path.exists(),
        "library key already exists at {}",
        path.display()
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create key directory {}", parent.display()))?;
    }
    let mut key = [0_u8; MASTER_KEY_BYTES];
    getrandom::fill(&mut key).context("OS random source unavailable")?;
    fs::write(path, key)
        .with_context(|| format!("Failed to write library key {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to restrict library key {}", path.display()))?;
    }
    Ok(key)
}

pub fn load_library_key(path: &Path) -> Result<[u8; 32]> {
    let bytes =
        fs::read(path).with_context(|| format!("Failed to read library key {}", path.display()))?;
    let key: [u8; 32] = bytes.try_into().map_err(|_| {
        anyhow::anyhow!(
            "library key at {} must contain exactly 32 bytes",
            path.display()
        )
    })?;
    Ok(key)
}

pub fn create_recipient_key(path: &Path) -> Result<[u8; 32]> {
    ensure!(
        !path.exists(),
        "recipient key already exists at {}",
        path.display()
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create key directory {}", parent.display()))?;
    }
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).context("OS random source unavailable")?;
    fs::write(path, secret)
        .with_context(|| format!("Failed to write recipient key {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to restrict recipient key {}", path.display()))?;
    }
    Ok(secret)
}

pub fn load_recipient_key(path: &Path) -> Result<[u8; 32]> {
    let bytes = fs::read(path)
        .with_context(|| format!("Failed to read recipient key {}", path.display()))?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("recipient key must contain exactly 32 bytes"))
}

pub fn recipient_public_key(secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

pub fn register_public_key(
    backend: &dyn CloudBackend,
    device_id: &str,
    public: &[u8; 32],
) -> Result<()> {
    backend.put(&format!("peers/{device_id}.pub"), public)
}

pub fn load_public_key(backend: &dyn CloudBackend, device_id: &str) -> Result<[u8; 32]> {
    let bytes = backend
        .get(&format!("peers/{device_id}.pub"))?
        .with_context(|| format!("device `{device_id}` has no registered public key"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("registered public key has invalid length"))
}

pub fn create_capability(
    backend: &dyn CloudBackend,
    library_key: &[u8; 32],
    recipient_public: &[u8; 32],
    scope: CapabilityScope,
    id: &str,
) -> Result<Capability> {
    let catalog = read_catalog(backend, library_key)?;
    let sessions = sessions_in_scope(&catalog, &scope)
        .into_iter()
        .map(|entry| ScopedSession {
            session_key: session_key(library_key, &entry.provider, &entry.session_id),
            entry,
        })
        .collect();
    let payload = CapabilityPayload { sessions };
    let mut ephemeral_bytes = [0_u8; 32];
    getrandom::fill(&mut ephemeral_bytes).context("OS random source unavailable")?;
    let ephemeral = StaticSecret::from(ephemeral_bytes);
    let shared = ephemeral.diffie_hellman(&PublicKey::from(*recipient_public));
    let wrapping_key = capability_key(shared.as_bytes(), id);
    let plaintext =
        serde_json::to_vec(&payload).context("Failed to serialize capability payload")?;
    let wrapped_payload = encrypt(&plaintext, &wrapping_key, &capability_aad(id))?;
    let capability = Capability {
        schema_version: SCHEMA_VERSION,
        id: id.to_string(),
        scope,
        wrapped_payload,
        ephemeral_public: PublicKey::from(&ephemeral).to_bytes(),
        created_at: now_rfc3339(),
        expires_at: None,
    };
    backend.put(
        &format!("caps/{id}.enc"),
        &serde_json::to_vec(&capability).context("Failed to serialize capability")?,
    )?;
    Ok(capability)
}

pub fn read_capability(backend: &dyn CloudBackend, id: &str) -> Result<Option<Capability>> {
    let Some(bytes) = backend.get(&format!("caps/{id}.enc"))? else {
        return Ok(None);
    };
    let capability: Capability =
        serde_json::from_slice(&bytes).context("Failed to parse capability")?;
    Ok(Some(capability))
}

pub fn unwrap_capability(
    capability: &Capability,
    recipient_secret: &[u8; 32],
) -> Result<CapabilityPayload> {
    let secret = StaticSecret::from(*recipient_secret);
    let public = PublicKey::from(capability.ephemeral_public);
    let shared = secret.diffie_hellman(&public);
    let wrapping_key = capability_key(shared.as_bytes(), &capability.id);
    let bytes = decrypt(
        &capability.wrapped_payload,
        &wrapping_key,
        &capability_aad(&capability.id),
    )?;
    serde_json::from_slice(&bytes).context("Failed to parse capability payload")
}

pub fn revoke_capability(backend: &dyn CloudBackend, id: &str) -> Result<()> {
    backend.delete(&format!("caps/{id}.enc"))
}

pub fn sessions_in_scope(catalog: &Catalog, scope: &CapabilityScope) -> Vec<SessionEntry> {
    catalog
        .sessions
        .iter()
        .filter(|entry| match scope {
            CapabilityScope::All => true,
            CapabilityScope::Session(session_id) => entry.session_id == *session_id,
            CapabilityScope::Provider(provider) => entry.provider == *provider,
        })
        .cloned()
        .collect()
}

pub fn cross_session_matches<'a>(catalog: &'a Catalog, query: &str) -> Vec<&'a SessionEntry> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    catalog
        .sessions
        .iter()
        .filter(|entry| {
            entry.preview.to_ascii_lowercase().contains(&needle)
                || entry
                    .title
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&needle)
                || entry.session_id.to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

pub fn session_key(library_key: &[u8; 32], provider: &str, session_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"sivtr-cloud-session-v1");
    hasher.update(library_key);
    hasher.update(b"\0");
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(session_id.as_bytes());
    hasher.finalize().into()
}

fn capability_key(shared_secret: &[u8], id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(shared_secret);
    hasher.update(b"sivtr-capability-v1");
    hasher.update(id.as_bytes());
    hasher.finalize().into()
}

fn capability_aad(id: &str) -> String {
    format!("sivtr-capability-v{SCHEMA_VERSION}:{id}")
}

pub fn read_catalog(backend: &dyn CloudBackend, key: &[u8; 32]) -> Result<Catalog> {
    let Some(envelope) = backend.get(CATALOG_KEY)? else {
        return Ok(Catalog::default());
    };
    let plaintext = decrypt(&envelope, key, &manifest_aad())?;
    let json = decompress(&plaintext).context("Failed to decompress cloud catalog")?;
    let manifest: Catalog =
        serde_json::from_slice(&json).context("Failed to parse cloud catalog")?;
    ensure!(
        manifest.schema_version == SCHEMA_VERSION,
        "unsupported cloud catalog schema version {}",
        manifest.schema_version
    );
    Ok(manifest)
}

pub fn sync_push(
    conn: &Connection,
    backend: &dyn CloudBackend,
    key: &[u8; 32],
    device_id: &str,
) -> Result<SyncPushReport> {
    let previous = read_catalog(backend, key)?;
    let exported = export_sessions(conn)?;
    let mut sessions_by_key: std::collections::BTreeMap<(String, String), SessionEntry> = previous
        .sessions
        .iter()
        .cloned()
        .map(|entry| ((entry.provider.clone(), entry.session_id.clone()), entry))
        .collect();

    let mut chunks_uploaded = 0usize;
    let mut chunks_reused = 0usize;
    let mut bytes_uploaded = 0u64;
    let mut sessions_written = 0usize;

    for session in exported {
        let map_key = (session.provider.clone(), session.session_id.clone());
        if let Some(old) = sessions_by_key.get(&map_key) {
            if old.content_hash == session.content_hash {
                continue;
            }
        }
        let chunks = upload_chunks(
            backend,
            &session_key(key, &session.provider, &session.session_id),
            &session.session_id,
            &session.canonical,
            &mut chunks_uploaded,
            &mut chunks_reused,
            &mut bytes_uploaded,
        )?;
        sessions_written += 1;
        sessions_by_key.insert(
            map_key,
            SessionEntry {
                provider: session.provider,
                session_id: session.session_id,
                source_path: session.source_path,
                cwd: session.cwd,
                workspace_key: session.workspace_key,
                title: session.title,
                project: session.project,
                starred: session.starred,
                started_at: session.started_at,
                ended_at: session.ended_at,
                content_hash: session.content_hash,
                record_count: session.record_count,
                preview: session.preview,
                chunks,
            },
        );
    }
    let sessions: Vec<SessionEntry> = sessions_by_key.into_values().collect();

    let generation = previous.generation.saturating_add(1);
    let manifest = Catalog {
        schema_version: SCHEMA_VERSION,
        device_id: device_id.to_string(),
        generation,
        created_at: now_rfc3339(),
        sessions,
    };
    let plaintext =
        compress(&serde_json::to_vec(&manifest).context("Failed to serialize cloud catalog")?)?;
    let envelope = encrypt(&plaintext, key, &manifest_aad())?;
    backend.put(CATALOG_KEY, &envelope)?;

    Ok(SyncPushReport {
        generation,
        sessions_total: manifest.sessions.len(),
        sessions_written,
        chunks_uploaded,
        chunks_reused,
        bytes_uploaded,
    })
}

pub fn sync_pull(
    conn: &Connection,
    backend: &dyn CloudBackend,
    key: &[u8; 32],
) -> Result<SyncPullReport> {
    sync_pull_scope(conn, backend, key, &CapabilityScope::All)
}

pub fn sync_pull_scope(
    conn: &Connection,
    backend: &dyn CloudBackend,
    key: &[u8; 32],
    scope: &CapabilityScope,
) -> Result<SyncPullReport> {
    let manifest = read_catalog(backend, key)?;
    ensure!(
        manifest.generation > 0,
        "no cloud catalog exists at this remote"
    );

    let mut added = 0usize;
    let mut unchanged = 0usize;
    let mut records_added = 0usize;
    let mut conflicts = Vec::new();
    let entries = sessions_in_scope(&manifest, scope);

    for entry in &entries {
        let local =
            store::load_records_by_key(conn, &entry.provider, &entry.session_id, BlobMode::Full)?;
        match local {
            Some(records) => {
                let local_json = canonical_jsonl(&records)?;
                if sha256_hex(&local_json) == entry.content_hash {
                    unchanged += 1;
                } else {
                    conflicts.push(format!(
                        "{}/{} differs locally",
                        entry.provider, entry.session_id
                    ));
                }
            }
            None => {
                let json = download_session(
                    backend,
                    &session_key(key, &entry.provider, &entry.session_id),
                    entry,
                )?;
                let records: Vec<WorkRecord> = parse_jsonl(&json, &entry.session_id)?;
                ensure!(
                    records.len() == entry.record_count,
                    "cloud session {}/{} declares {} records but contains {}",
                    entry.provider,
                    entry.session_id,
                    entry.record_count,
                    records.len()
                );
                ensure!(
                    sha256_hex(&json) == entry.content_hash,
                    "cloud session {}/{} content hash mismatch",
                    entry.provider,
                    entry.session_id
                );
                let cwd = entry.cwd.as_deref().unwrap_or_default();
                let workspace_key = entry.workspace_key.clone();
                store::upsert_session(
                    conn,
                    &SessionUpsert {
                        provider: &entry.provider,
                        session_id: &entry.session_id,
                        source_path: Path::new(&entry.source_path),
                        cwd: entry.cwd.as_deref(),
                        workspace_key: &workspace_key,
                        title: entry.title.as_deref(),
                        stamp: (0, 0, 0),
                        fingerprint: 0,
                        records: &records,
                        usage_events: &[],
                    },
                )
                .with_context(|| {
                    format!(
                        "Failed to restore cloud session {}/{}",
                        entry.provider, entry.session_id
                    )
                })?;
                if entry.starred {
                    store::set_session_starred(conn, &entry.provider, &entry.session_id, true)?;
                }
                let _ = (cwd, workspace_key);
                added += 1;
                records_added += records.len();
            }
        }
    }

    Ok(SyncPullReport {
        generation: manifest.generation,
        sessions_added: added,
        sessions_unchanged: unchanged,
        records_added,
        conflicts,
    })
}

pub fn sync_pull_capability(
    conn: &Connection,
    backend: &dyn CloudBackend,
    payload: &CapabilityPayload,
) -> Result<SyncPullReport> {
    let mut added = 0usize;
    let mut unchanged = 0usize;
    let mut records_added = 0usize;
    let mut conflicts = Vec::new();

    for scoped in &payload.sessions {
        let entry = &scoped.entry;
        match store::load_records_by_key(conn, &entry.provider, &entry.session_id, BlobMode::Full)?
        {
            Some(records) => {
                if sha256_hex(&canonical_jsonl(&records)?) == entry.content_hash {
                    unchanged += 1;
                } else {
                    conflicts.push(format!(
                        "{}/{} differs locally",
                        entry.provider, entry.session_id
                    ));
                }
            }
            None => {
                let json = download_session(backend, &scoped.session_key, entry)?;
                let records = parse_jsonl(&json, &entry.session_id)?;
                ensure!(
                    records.len() == entry.record_count,
                    "cloud session {}/{} declares {} records but contains {}",
                    entry.provider,
                    entry.session_id,
                    entry.record_count,
                    records.len()
                );
                ensure!(
                    sha256_hex(&json) == entry.content_hash,
                    "cloud session {}/{} content hash mismatch",
                    entry.provider,
                    entry.session_id
                );
                let workspace_key = entry.workspace_key.clone();
                store::upsert_session(
                    conn,
                    &SessionUpsert {
                        provider: &entry.provider,
                        session_id: &entry.session_id,
                        source_path: Path::new(&entry.source_path),
                        cwd: entry.cwd.as_deref(),
                        workspace_key: &workspace_key,
                        title: entry.title.as_deref(),
                        stamp: (0, 0, 0),
                        fingerprint: 0,
                        records: &records,
                        usage_events: &[],
                    },
                )
                .with_context(|| {
                    format!(
                        "Failed to restore cloud session {}/{}",
                        entry.provider, entry.session_id
                    )
                })?;
                if entry.starred {
                    store::set_session_starred(conn, &entry.provider, &entry.session_id, true)?;
                }
                added += 1;
                records_added += records.len();
            }
        }
    }

    Ok(SyncPullReport {
        generation: 0,
        sessions_added: added,
        sessions_unchanged: unchanged,
        records_added,
        conflicts,
    })
}

pub fn reclaim_cache(
    conn: &Connection,
    backend: &dyn CloudBackend,
    key: &[u8; 32],
    scope: &CapabilityScope,
) -> Result<usize> {
    let catalog = read_catalog(backend, key)?;
    let keep: std::collections::BTreeSet<(String, String)> = sessions_in_scope(&catalog, scope)
        .into_iter()
        .map(|entry| (entry.provider, entry.session_id))
        .collect();
    let mut removed = 0usize;
    for row in store::list_session_rows(conn, None, None, None)? {
        if keep.contains(&(row.provider.clone(), row.session_id.clone())) {
            continue;
        }
        conn.execute("DELETE FROM records WHERE session_row = ?1", [row.row_id])
            .context("Failed to remove cached records")?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", [row.row_id])
            .context("Failed to remove cached session")?;
        removed += 1;
    }
    Ok(removed)
}

pub fn verify_remote(backend: &dyn CloudBackend, key: &[u8; 32]) -> Result<VerifyReport> {
    let manifest = read_catalog(backend, key)?;
    let mut chunks_checked = 0usize;
    let mut bytes_verified = 0u64;
    for entry in &manifest.sessions {
        let session_key = session_key(key, &entry.provider, &entry.session_id);
        let mut session_bytes = Vec::new();
        for chunk in &entry.chunks {
            let object_key = chunk_key(&chunk.hash)?;
            let envelope = backend
                .get(&object_key)?
                .with_context(|| format!("cloud chunk {} is missing", chunk.hash))?;
            let plaintext = decrypt(
                &envelope,
                &session_key,
                &chunk_aad(&entry.session_id, &chunk.hash),
            )
            .with_context(|| format!("Failed to decrypt cloud chunk {}", chunk.hash))?;
            let bytes = decompress(&plaintext)
                .with_context(|| format!("Failed to decompress cloud chunk {}", chunk.hash))?;
            ensure!(
                sha256_hex(&bytes) == chunk.hash,
                "cloud chunk {} hash mismatch",
                chunk.hash
            );
            session_bytes.extend_from_slice(&bytes);
            chunks_checked += 1;
            bytes_verified += bytes.len() as u64;
        }
        ensure!(
            sha256_hex(&session_bytes) == entry.content_hash,
            "cloud session {}/{} content hash mismatch",
            entry.provider,
            entry.session_id
        );
    }
    Ok(VerifyReport {
        generation: manifest.generation,
        sessions: manifest.sessions.len(),
        chunks_checked,
        bytes_verified,
    })
}

pub fn sync_status(
    conn: &Connection,
    backend: &dyn CloudBackend,
    key: &[u8; 32],
) -> Result<SyncStatus> {
    let manifest = read_catalog(backend, key)?;
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    let mut local_sessions = 0usize;

    for row in store::list_session_rows(conn, None, None, None)? {
        local_sessions += 1;
        let label = format!("{}/{}", row.provider, row.session_id);
        let Some(entry) = manifest
            .sessions
            .iter()
            .find(|entry| entry.provider == row.provider && entry.session_id == row.session_id)
        else {
            missing.push(label);
            continue;
        };
        let records =
            store::load_records_by_key(conn, &row.provider, &row.session_id, BlobMode::Full)?
                .unwrap_or_default();
        if sha256_hex(&canonical_jsonl(&records)?) != entry.content_hash {
            changed.push(label);
        }
    }

    Ok(SyncStatus {
        generation: manifest.generation,
        created_at: manifest.created_at,
        device_id: manifest.device_id,
        local_sessions,
        remote_sessions: manifest.sessions.len(),
        missing,
        changed,
    })
}

#[derive(Debug, Clone)]
struct ExportedSession {
    provider: String,
    session_id: String,
    source_path: String,
    cwd: Option<String>,
    workspace_key: String,
    title: Option<String>,
    project: String,
    starred: bool,
    started_at: Option<String>,
    ended_at: Option<String>,
    content_hash: String,
    record_count: usize,
    preview: String,
    canonical: Vec<u8>,
}

fn export_sessions(conn: &Connection) -> Result<Vec<ExportedSession>> {
    let rows = store::list_session_rows(conn, None, None, None)?;
    let mut exported = Vec::with_capacity(rows.len());
    for row in rows {
        let meta = store::session_meta_by_key(conn, &row.provider, &row.session_id)?.with_context(
            || {
                format!(
                    "archived session {}/{} has no metadata",
                    row.provider, row.session_id
                )
            },
        )?;
        let records =
            store::load_records_by_key(conn, &row.provider, &row.session_id, BlobMode::Full)?
                .with_context(|| {
                    format!(
                        "archived session {}/{} disappeared",
                        row.provider, row.session_id
                    )
                })?;
        let canonical = canonical_jsonl(&records)?;
        let preview_text = records
            .iter()
            .map(|record| record.combined_text())
            .collect::<Vec<_>>()
            .join("\n");
        let preview: String = preview_text.chars().take(4096).collect();
        let workspace_key = meta
            .cwd
            .as_deref()
            .map(Path::new)
            .and_then(workspace::repo_identity)
            .unwrap_or_default();
        exported.push(ExportedSession {
            provider: row.provider,
            session_id: row.session_id,
            source_path: row.source_path,
            cwd: meta.cwd,
            workspace_key,
            title: meta.title,
            project: meta.project,
            starred: meta.starred,
            started_at: meta.started_at,
            ended_at: meta.ended_at,
            content_hash: sha256_hex(&canonical),
            record_count: records.len(),
            preview,
            canonical,
        });
    }
    exported.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(exported)
}

fn canonical_jsonl(records: &[WorkRecord]) -> Result<Vec<u8>> {
    let mut ordered: Vec<&WorkRecord> = records.iter().collect();
    ordered.sort_by_cached_key(|record| record.work_ref.to_string());
    let mut output = Vec::new();
    for record in ordered {
        serde_json::to_writer(&mut output, record).context("Failed to serialize cloud record")?;
        output.push(b'\n');
    }
    Ok(output)
}

fn parse_jsonl(bytes: &[u8], session_id: &str) -> Result<Vec<WorkRecord>> {
    let text = std::str::from_utf8(bytes).context("Backup session is not valid UTF-8")?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).with_context(|| {
                format!(
                    "Failed to parse record {} of cloud session {session_id}",
                    index + 1
                )
            })
        })
        .collect()
}

fn upload_chunks(
    backend: &dyn CloudBackend,
    key: &[u8; 32],
    session_id: &str,
    jsonl: &[u8],
    uploaded: &mut usize,
    reused: &mut usize,
    bytes_uploaded: &mut u64,
) -> Result<Vec<ChunkRef>> {
    let mut chunks = Vec::new();
    for piece in jsonl.chunks(DEFAULT_CHUNK_BYTES) {
        let hash = sha256_hex(piece);
        let object_key = chunk_key(&hash)?;
        let compressed = compress(piece)?;
        let envelope = encrypt(&compressed, key, &chunk_aad(session_id, &hash))?;
        if backend.put_if_absent(&object_key, &envelope)? {
            *uploaded += 1;
            *bytes_uploaded += envelope.len() as u64;
        } else {
            *reused += 1;
        }
        chunks.push(ChunkRef {
            hash,
            bytes: piece.len() as u64,
        });
    }
    Ok(chunks)
}

fn download_session(
    backend: &dyn CloudBackend,
    key: &[u8; 32],
    entry: &SessionEntry,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    for chunk in &entry.chunks {
        let object_key = chunk_key(&chunk.hash)?;
        let envelope = backend
            .get(&object_key)?
            .with_context(|| format!("cloud chunk {} is missing", chunk.hash))?;
        let plaintext = decrypt(&envelope, key, &chunk_aad(&entry.session_id, &chunk.hash))?;
        output.extend_from_slice(
            &decompress(&plaintext)
                .with_context(|| format!("Failed to decompress cloud chunk {}", chunk.hash))?,
        );
    }
    Ok(output)
}

fn compress(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .context("Failed to compress cloud data")?;
    encoder
        .finish()
        .context("Failed to finish cloud compression")
}

fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .context("Failed to decompress cloud data")?;
    Ok(output)
}

fn encrypt(plaintext: &[u8], key: &[u8; 32], aad: &str) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0_u8; 12];
    getrandom::fill(&mut nonce_bytes).context("OS random source unavailable")?;
    let cipher_key = Key::<Aes256Gcm>::try_from(key.as_slice())
        .map_err(|_| anyhow::anyhow!("invalid library master key"))?;
    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("invalid cloud nonce"))?;
    let cipher = Aes256Gcm::new(&cipher_key);
    let sealed = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("AES-GCM cloud encryption failed"))?;
    let mut envelope = Vec::with_capacity(ENVELOPE_HEADER_BYTES + sealed.len());
    envelope.extend_from_slice(ENVELOPE_MAGIC);
    envelope.push(SCHEMA_VERSION as u8);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&sealed);
    Ok(envelope)
}

fn decrypt(envelope: &[u8], key: &[u8; 32], aad: &str) -> Result<Vec<u8>> {
    ensure!(
        envelope.len() >= ENVELOPE_HEADER_BYTES + TAG_BYTES,
        "cloud envelope is truncated"
    );
    ensure!(
        &envelope[..8] == ENVELOPE_MAGIC,
        "cloud envelope has an invalid header"
    );
    ensure!(
        envelope[8] == SCHEMA_VERSION as u8,
        "unsupported cloud envelope version {}",
        envelope[8]
    );
    let nonce_bytes = &envelope[9..21];
    let sealed = &envelope[ENVELOPE_HEADER_BYTES..];
    let cipher_key = Key::<Aes256Gcm>::try_from(key.as_slice())
        .map_err(|_| anyhow::anyhow!("invalid library master key"))?;
    let nonce = Nonce::try_from(nonce_bytes).map_err(|_| anyhow::anyhow!("invalid cloud nonce"))?;
    let cipher = Aes256Gcm::new(&cipher_key);
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: sealed,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| {
            anyhow::anyhow!("cloud decryption failed; check the master key and remote data")
        })
}

fn manifest_aad() -> String {
    format!("sivtr-cloud-catalog-v{SCHEMA_VERSION}")
}

fn chunk_aad(session_id: &str, hash: &str) -> String {
    format!("sivtr-cloud-chunk-v{SCHEMA_VERSION}:{session_id}:{hash}")
}

fn chunk_key(hash: &str) -> Result<String> {
    ensure!(hash.len() >= 2, "invalid cloud chunk hash");
    Ok(format!("objects/{}/{}", &hash[..2], hash))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    use std::fmt::Write as _;
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Debug, Clone)]
pub struct LocalDirBackend {
    root: PathBuf,
}

impl LocalDirBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn resolve(&self, key: &str) -> Result<PathBuf> {
        ensure!(
            !key.starts_with('/') && !key.starts_with('\\') && !key.contains('\\'),
            "invalid cloud object key"
        );
        let segments: Vec<&str> = key.split('/').collect();
        ensure!(
            !segments.is_empty()
                && segments
                    .iter()
                    .all(|segment| !segment.is_empty() && *segment != ".."),
            "invalid cloud object key"
        );
        Ok(self.root.join(segments.iter().collect::<PathBuf>()))
    }
}

impl CloudBackend for LocalDirBackend {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create cloud directory {}", parent.display())
            })?;
        }
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, bytes)
            .with_context(|| format!("Failed to write cloud object {}", path.display()))?;
        fs::rename(&temporary, &path)
            .with_context(|| format!("Failed to install cloud object {}", path.display()))?;
        Ok(())
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<bool> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create cloud directory {}", parent.display())
            })?;
        }
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to create cloud object {}", path.display()));
            }
        };
        file.write_all(bytes)
            .with_context(|| format!("Failed to write cloud object {}", path.display()))?;
        Ok(true)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.resolve(key)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .with_context(|| format!("Failed to read cloud object {}", path.display()))?;
        Ok(Some(bytes))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = self.resolve(key)?;
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to delete cloud object {}", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::schema::init_schema;
    use crate::archive::store::SessionUpsert;
    use crate::test_fixtures::terminal_record;
    use rusqlite::Connection;
    use std::collections::BTreeMap;
    use std::path::Path;

    struct MemoryBackend {
        objects: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl MemoryBackend {
        fn new() -> Self {
            Self {
                objects: std::sync::Mutex::new(BTreeMap::new()),
            }
        }
    }

    impl CloudBackend for MemoryBackend {
        fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
            self.objects
                .lock()
                .expect("mutex poisoned")
                .insert(key.to_string(), bytes.to_vec());
            Ok(())
        }

        fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<bool> {
            let mut objects = self.objects.lock().expect("mutex poisoned");
            if objects.contains_key(key) {
                return Ok(false);
            }
            objects.insert(key.to_string(), bytes.to_vec());
            Ok(true)
        }

        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            Ok(self
                .objects
                .lock()
                .expect("mutex poisoned")
                .get(key)
                .cloned())
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.objects.lock().expect("mutex poisoned").remove(key);
            Ok(())
        }
    }

    #[test]
    fn envelope_round_trip_uses_aad() {
        let key = [7_u8; 32];
        let envelope = encrypt(b"payload", &key, "aad").expect("encrypt");
        assert_eq!(
            decrypt(&envelope, &key, "aad").expect("decrypt"),
            b"payload"
        );
        assert!(decrypt(&envelope, &key, "other").is_err());
    }

    #[test]
    fn chunk_hash_is_content_addressed() {
        let first = sha256_hex(b"same");
        let second = sha256_hex(b"same");
        assert_eq!(first, second);
        assert_eq!(
            chunk_key(&first).expect("key"),
            format!("objects/{}/{}", &first[..2], first)
        );
    }

    #[test]
    fn sync_push_pull_and_verify_round_trip() {
        let conn = Connection::open_in_memory().expect("in-memory archive");
        init_schema(&conn).expect("archive schema");
        let records = vec![terminal_record("session-1", 1, "first", "cloud payload")];
        store::upsert_session(
            &conn,
            &SessionUpsert {
                provider: "terminal",
                session_id: "session-1",
                source_path: Path::new("memory://session-1"),
                cwd: None,
                workspace_key: "",
                title: Some("first"),
                stamp: (0, 0, 0),
                fingerprint: 0,
                records: &records,
                usage_events: &[],
            },
        )
        .expect("insert session");

        let backend = MemoryBackend::new();
        let key = [9_u8; 32];
        let pushed = sync_push(&conn, &backend, &key, "test-device").expect("push");
        assert_eq!(pushed.generation, 1);
        assert_eq!(pushed.sessions_written, 1);
        assert!(pushed.chunks_uploaded > 0);

        let verified = verify_remote(&backend, &key).expect("verify");
        assert_eq!(verified.generation, 1);
        assert_eq!(verified.sessions, 1);
        assert!(verified.chunks_checked > 0);

        conn.execute_batch("DELETE FROM records; DELETE FROM sessions;")
            .expect("clear archive");
        let pulled = sync_pull(&conn, &backend, &key).expect("pull");
        assert_eq!(pulled.sessions_added, 1);
        assert_eq!(pulled.records_added, 1);
        assert!(pulled.conflicts.is_empty());

        let restored = store::load_records_by_key(&conn, "terminal", "session-1", BlobMode::Full)
            .expect("load restored")
            .expect("restored session exists");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].title, "first");
    }

    #[test]
    fn stale_local_state_refreshes_from_catalog() {
        let conn = Connection::open_in_memory().expect("in-memory archive");
        init_schema(&conn).expect("archive schema");
        let records = vec![terminal_record("stale-session", 1, "stale", "recover me")];
        store::upsert_session(
            &conn,
            &SessionUpsert {
                provider: "terminal",
                session_id: "stale-session",
                source_path: Path::new("memory://stale-session"),
                cwd: None,
                workspace_key: "",
                title: None,
                stamp: (0, 0, 0),
                fingerprint: 0,
                records: &records,
                usage_events: &[],
            },
        )
        .expect("insert session");
        let backend = MemoryBackend::new();
        let key = [17_u8; 32];
        sync_push(&conn, &backend, &key, "device").expect("initial push");
        conn.execute_batch("DELETE FROM records; DELETE FROM sessions;")
            .expect("simulate stale local cache");
        sync_push(&conn, &backend, &key, "device").expect("push with stale cache");
        let pulled = sync_pull(&conn, &backend, &key).expect("refresh");
        assert_eq!(pulled.sessions_added, 1);
        assert!(
            store::load_records_by_key(&conn, "terminal", "stale-session", BlobMode::Full)
                .expect("load")
                .is_some()
        );
    }

    #[test]
    fn repeated_push_is_idempotent() {
        let conn = Connection::open_in_memory().expect("in-memory archive");
        init_schema(&conn).expect("archive schema");
        let records = vec![terminal_record("session-2", 1, "second", "stable payload")];
        store::upsert_session(
            &conn,
            &SessionUpsert {
                provider: "terminal",
                session_id: "session-2",
                source_path: Path::new("memory://session-2"),
                cwd: None,
                workspace_key: "",
                title: Some("second"),
                stamp: (0, 0, 0),
                fingerprint: 0,
                records: &records,
                usage_events: &[],
            },
        )
        .expect("insert session");
        let backend = MemoryBackend::new();
        let key = [3_u8; 32];
        let first = sync_push(&conn, &backend, &key, "device").expect("first push");
        let second = sync_push(&conn, &backend, &key, "device").expect("second push");
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
        assert_eq!(second.sessions_written, 0);
        assert_eq!(second.chunks_uploaded, 0);
    }

    #[test]
    fn catalog_search_spans_multiple_sessions() {
        let mut catalog = Catalog::default();
        catalog.sessions.push(SessionEntry {
            provider: "terminal".into(),
            session_id: "one".into(),
            source_path: "memory://one".into(),
            cwd: None,
            workspace_key: String::new(),
            title: None,
            project: String::new(),
            starred: false,
            started_at: None,
            ended_at: None,
            content_hash: "a".repeat(64),
            record_count: 1,
            preview: "alpha marker".into(),
            chunks: Vec::new(),
        });
        catalog.sessions.push(SessionEntry {
            provider: "terminal".into(),
            session_id: "two".into(),
            source_path: "memory://two".into(),
            cwd: None,
            workspace_key: String::new(),
            title: None,
            project: String::new(),
            starred: false,
            started_at: None,
            ended_at: None,
            content_hash: "b".repeat(64),
            record_count: 1,
            preview: "beta marker".into(),
            chunks: Vec::new(),
        });
        let matches = cross_session_matches(&catalog, "marker");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn capability_scope_and_revocation() {
        let conn = Connection::open_in_memory().expect("in-memory archive");
        init_schema(&conn).expect("archive schema");
        for (session_id, text) in [
            ("session-1", "scoped payload"),
            ("session-2", "other payload"),
        ] {
            let records = vec![terminal_record(session_id, 1, session_id, text)];
            store::upsert_session(
                &conn,
                &SessionUpsert {
                    provider: "terminal",
                    session_id,
                    source_path: Path::new(&format!("memory://{session_id}")),
                    cwd: None,
                    workspace_key: "",
                    title: None,
                    stamp: (0, 0, 0),
                    fingerprint: 0,
                    records: &records,
                    usage_events: &[],
                },
            )
            .expect("insert session");
        }

        let backend = MemoryBackend::new();
        let library_key = [11_u8; 32];
        let recipient_secret = [12_u8; 32];
        let recipient_public = recipient_public_key(&recipient_secret);
        register_public_key(&backend, "alice", &recipient_public).expect("register");
        let fetched = load_public_key(&backend, "alice").expect("load public");
        assert_eq!(fetched, recipient_public);
        sync_push(&conn, &backend, &library_key, "owner").expect("push");

        let _capability = create_capability(
            &backend,
            &library_key,
            &fetched,
            CapabilityScope::Session("session-1".into()),
            "cap-1",
        )
        .expect("create capability");
        let stored = read_capability(&backend, "cap-1")
            .expect("read capability")
            .expect("capability exists");
        let payload = unwrap_capability(&stored, &recipient_secret).expect("unwrap");
        assert_eq!(payload.sessions.len(), 1);
        assert_eq!(payload.sessions[0].entry.session_id, "session-1");
        assert_eq!(
            payload.sessions[0].session_key,
            session_key(&library_key, "terminal", "session-1")
        );
        assert!(unwrap_capability(&stored, &[13_u8; 32]).is_err());

        let catalog = read_catalog(&backend, &library_key).expect("catalog");
        let out_of_scope = catalog
            .sessions
            .iter()
            .find(|entry| entry.session_id == "session-2")
            .expect("out of scope session");
        assert!(
            download_session(&backend, &payload.sessions[0].session_key, out_of_scope).is_err(),
            "scoped key must not decrypt another session"
        );

        revoke_capability(&backend, "cap-1").expect("revoke");
        assert!(read_capability(&backend, "cap-1")
            .expect("read revoked")
            .is_none());
    }

    #[test]
    fn reclaim_cache_removes_sessions_outside_scope() {
        let conn = Connection::open_in_memory().expect("in-memory archive");
        init_schema(&conn).expect("archive schema");
        for (session_id, text) in [("keep", "keep payload"), ("drop", "drop payload")] {
            let records = vec![terminal_record(session_id, 1, session_id, text)];
            store::upsert_session(
                &conn,
                &SessionUpsert {
                    provider: "terminal",
                    session_id,
                    source_path: Path::new(&format!("memory://{session_id}")),
                    cwd: None,
                    workspace_key: "",
                    title: None,
                    stamp: (0, 0, 0),
                    fingerprint: 0,
                    records: &records,
                    usage_events: &[],
                },
            )
            .expect("insert session");
        }
        let backend = MemoryBackend::new();
        let key = [5_u8; 32];
        sync_push(&conn, &backend, &key, "device").expect("push");
        let removed = reclaim_cache(
            &conn,
            &backend,
            &key,
            &CapabilityScope::Session("keep".into()),
        )
        .expect("reclaim");
        assert_eq!(removed, 1);
        assert!(
            store::load_records_by_key(&conn, "terminal", "keep", BlobMode::Full)
                .expect("load keep")
                .is_some()
        );
        assert!(
            store::load_records_by_key(&conn, "terminal", "drop", BlobMode::Full)
                .expect("load drop")
                .is_none()
        );
    }
}
