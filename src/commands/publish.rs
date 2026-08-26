//! Browser publication: local projection, client-side encryption, and the
//! small local registry needed to revoke bearer links later.

use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use flate2::{write::GzEncoder, Compression};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sivtr_core::record::{work_atoms, WorkRecord, WorkRef};
use sivtr_core::{
    config::SivtrConfig,
    publication::{
        create_publication_draft, PublicConversationSnapshot, PublicationDraft, PublicationExpiry,
        PublicationPolicy,
    },
    workspace,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::time::Duration;
use std::time::UNIX_EPOCH;

use crate::cli::{
    PublishAction, PublishCommand, PublishCreateArgs, PublishFormat, PublishIdArgs,
    PublishListArgs, PublishPreviewArgs, PublishRevokeArgs,
};
use crate::commands::memory::{filter::Filter, workset};
use crate::output;
use crate::tui::workspace::{WorkspaceFocus, WorkspaceSession, WorkspaceSource};

const ENVELOPE_LIMIT: usize = 5 * 1024 * 1024;
const SNAPSHOT_PLAINTEXT_LIMIT: usize = 16 * 1024 * 1024;
const ENVELOPE_MAGIC: &[u8; 8] = b"SIVTPUB1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationStatus {
    Pending,
    Active,
    Revoked,
    Expired,
    Failed,
}

impl PublicationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for PublicationStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            _ => bail!("unknown publication status `{value}`"),
        }
    }
}

#[derive(Debug, Clone)]
struct PublicationRow {
    id: String,
    endpoint: String,
    viewer_key: String,
    management_token: String,
    title: String,
    provider: String,
    source_refs: String,
    content_sha256: String,
    redaction_count: i64,
    warning_count: i64,
    created_at: String,
    expires_at: String,
    status: PublicationStatus,
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct PublicationListItem {
    publication_id: String,
    title: String,
    provider: String,
    status: String,
    created_at: String,
    expires_at: String,
    redaction_count: i64,
    warning_count: i64,
    content_sha256: String,
    last_error: Option<String>,
}

pub fn execute(command: PublishCommand) -> Result<()> {
    match command.action {
        PublishAction::Preview(args) => preview(args),
        PublishAction::Create(args) => create(args),
        PublishAction::List(args) => list(args),
        PublishAction::Link(args) => link(args),
        PublishAction::Revoke(args) => revoke(args),
    }
}

fn load_publication_set(source: &str) -> Result<workset::WorkSet> {
    // Preview must be offline.  Named remote/group scopes are rejected before
    // the unified WorkSet query has a chance to start a daemon or dial a peer.
    if source.contains(':') && !source.starts_with("local:") {
        bail!("publish v1 accepts local WorkSets only; remote/group scopes are not publishable");
    }
    let mut set = workset::query(source, Filter::none(), None)
        .with_context(|| format!("failed to resolve publication source `{source}`"))?;
    set.materialize_parts()?;
    Ok(set)
}

fn load_draft(source: &str, title: Option<String>, expiry: &str) -> Result<PublicationDraft> {
    let mut set = load_publication_set(source)?;
    load_draft_from_set(&mut set, title, expiry)
}

fn load_draft_from_set(
    set: &mut workset::WorkSet,
    title: Option<String>,
    expiry: &str,
) -> Result<PublicationDraft> {
    let expires = PublicationExpiry::parse(expiry)?;
    set.materialize_parts()?;
    create_publication_draft(
        &set.records,
        &set.anchors(),
        &PublicationPolicy {
            title,
            expires,
            published_at: None,
        },
    )
}

fn pick_publication_set(source: &str) -> Result<workset::WorkSet> {
    let set = load_publication_set(source)?;
    let first = set
        .records
        .first()
        .ok_or_else(|| anyhow::anyhow!("publication source contains no records"))?;
    let provider = first
        .work_ref
        .provider()
        .ok_or_else(|| anyhow::anyhow!("publication picker only supports agent sessions"))?;
    let session_id = first.session.id.clone();
    ensure!(
        first.kind == sivtr_core::record::WorkRecordKind::ChatTurn && first.work_ref.is_local(),
        "publication picker only supports one local agent session"
    );
    ensure!(
        set.records.iter().all(|record| {
            record.kind == sivtr_core::record::WorkRecordKind::ChatTurn
                && record.work_ref.is_local()
                && record.work_ref.provider() == Some(provider)
                && record.session.id == session_id
        }),
        "publication picker requires exactly one local agent session"
    );

    let workspace_source = WorkspaceSource::agent(provider);
    let session = WorkspaceSession {
        source: workspace_source.clone(),
        session_id,
        modified: UNIX_EPOCH,
        title: first.title.clone(),
        search_title: first.title.clone(),
        records: set.records.clone(),
        body_loaded: true,
    };
    let picked = crate::commands::browse::run_with_sessions(
        workspace_source,
        vec![session],
        WorkspaceFocus::Dialogues,
    )?;
    ensure!(!picked.anchors.is_empty(), "publication selection is empty");
    let anchors = expand_picker_anchors(&set.records, &picked.anchors)?;
    ensure!(!anchors.is_empty(), "publication selection is empty");
    Ok(publication_workset(set, anchors))
}

fn publication_workset(set: workset::WorkSet, anchors: Vec<WorkRef>) -> workset::WorkSet {
    let records = workset::records_for_anchors(&set.records, &anchors);
    workset::WorkSet::with_anchors(set.cwd, records, anchors)
}

fn expand_picker_anchors(records: &[WorkRecord], picked: &[WorkRef]) -> Result<Vec<WorkRef>> {
    let mut selected: BTreeMap<String, (usize, BTreeSet<usize>)> = BTreeMap::new();
    for anchor in picked {
        let record_index = records
            .iter()
            .position(|record| record.work_ref.whole() == anchor.whole())
            .ok_or_else(|| anyhow::anyhow!("picker anchor `{anchor}` has no record"))?;
        let record = &records[record_index];
        let entry = selected
            .entry(record.work_ref.whole().to_string())
            .or_insert_with(|| (record_index, BTreeSet::new()));
        let mut atoms = work_atoms(record, true);
        atoms.extend(work_atoms(record, false));
        if let Some(seq) = anchor.part() {
            let atom = atoms
                .iter()
                .find(|atom| atom.part_seqs.contains(&seq))
                .ok_or_else(|| anyhow::anyhow!("picker anchor `{anchor}` has no atom"))?;
            entry.1.extend(atom.part_seqs.iter().copied());
        } else {
            entry
                .1
                .extend(atoms.into_iter().flat_map(|atom| atom.part_seqs));
        }
    }

    let mut groups = selected.into_values().collect::<Vec<_>>();
    groups.sort_by_key(|(record_index, _)| records[*record_index].work_ref.index());
    let mut anchors = Vec::new();
    for (record_index, selected_parts) in groups {
        let record = &records[record_index];
        let mut seqs = selected_parts.into_iter().collect::<Vec<_>>();
        seqs.sort_unstable();
        anchors.extend(seqs.into_iter().map(|seq| record.work_ref.with_part(seq)));
    }
    Ok(anchors)
}

fn preview(args: PublishPreviewArgs) -> Result<()> {
    if args.save.is_some() && !args.pick {
        bail!("publish preview --save requires --pick");
    }
    let draft = if args.pick {
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            bail!("publish preview --pick requires an interactive terminal");
        }
        let mut set = pick_publication_set(&args.source)?;
        if let Some(name) = args.save.as_deref() {
            set.save_as(name)?;
            output::success(format!("saved @{name}"));
        }
        load_draft_from_set(&mut set, args.title, &args.expires)?
    } else {
        load_draft(&args.source, args.title, &args.expires)?
    };
    match args.format {
        PublishFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&draft.snapshot)?);
            print_risks(&draft);
        }
        PublishFormat::Human => {
            print!("{}", format_human_preview_meta(&draft));
            println!();
            print_snapshot_items(&draft.snapshot);
        }
    }
    Ok(())
}

fn print_snapshot_items(snapshot: &PublicConversationSnapshot) {
    match snapshot {
        PublicConversationSnapshot::V1(snapshot) => {
            for item in &snapshot.items {
                println!(
                    "[{}]",
                    match item.role {
                        sivtr_core::publication::PublicRole::User => "User",
                        sivtr_core::publication::PublicRole::Assistant => "Assistant",
                    }
                );
                println!("{}", item.text);
                println!();
            }
        }
        PublicConversationSnapshot::V2(snapshot) => {
            for item in &snapshot.items {
                if item.gap_before {
                    println!("[部分内容未分享]");
                    println!();
                }
                let label = item
                    .label
                    .as_deref()
                    .map(|label| format!(" ({label})"))
                    .unwrap_or_default();
                println!("[{:?}{}]", item.kind, label);
                for part in &item.parts {
                    if part.gap_before {
                        println!("[部分内容未分享]");
                        println!();
                    }
                    println!("{}", part.text);
                }
                println!();
                if item.gap_after {
                    println!("[部分内容未分享]");
                    println!();
                }
            }
        }
    }
}

fn create(args: PublishCreateArgs) -> Result<()> {
    let draft = load_draft(&args.source, args.title, &args.expires)?;
    let envelope_preview = compress_snapshot(&draft)?;
    let envelope_size = envelope_preview
        .len()
        .checked_add(8 + 2 + 12 + 16)
        .ok_or_else(|| anyhow::anyhow!("encrypted publication envelope size overflow"))?;
    if envelope_size > ENVELOPE_LIMIT {
        bail!(
            "encrypted publication envelope is {} bytes; maximum is 5 MiB; narrow the WorkSet",
            envelope_size
        );
    }
    let has_warnings = draft.risks.iter().any(|risk| is_warning_only(&risk.kind));
    print_create_summary(&draft, &args.expires, envelope_size);
    if has_warnings {
        output::warning("存在未自动处理的路径、邮箱或内网地址风险；请确认公开内容");
    }
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !args.yes {
        if !interactive {
            bail!("non-interactive publish requires --yes");
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("创建只读公开链接？")
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("publication cancelled");
        }
    }
    require_allow_warnings(has_warnings, args.allow_warnings)?;

    let config = SivtrConfig::load()?;
    let endpoint = resolve_endpoint(&config)?;
    let expiry = PublicationExpiry::parse(&args.expires)?;
    let id = format!("{}_{}", expiry.as_str(), random_token(16)?);
    let viewer_key = random_token(32)?;
    let management_token = random_token(32)?;
    let now = Utc::now().to_rfc3339();
    let expires_at = draft.snapshot.expires_at().to_string();
    let mut db = PublicationDb::open()?;
    db.insert_pending(&PublicationRow {
        id: id.clone(),
        endpoint: endpoint.clone(),
        viewer_key: viewer_key.clone(),
        management_token: management_token.clone(),
        title: draft.snapshot.title().to_string(),
        provider: draft.snapshot.provider().to_string(),
        source_refs: serde_json::to_string(&draft.source_refs)?,
        content_sha256: draft.content_sha256.clone(),
        redaction_count: draft.redaction_count as i64,
        warning_count: draft
            .risks
            .iter()
            .filter(|risk| is_warning_only(&risk.kind))
            .map(|risk| risk.count as i64)
            .sum(),
        created_at: now,
        expires_at,
        status: PublicationStatus::Pending,
        last_error: None,
    })?;
    let envelope = match encrypt_snapshot(&draft, &id, &viewer_key) {
        Ok(value) => value,
        Err(error) => {
            let _ = db.mark_failed(&id, &error.to_string());
            return Err(error);
        }
    };
    if let Err(error) = upload(&endpoint, &id, &management_token, &envelope) {
        let _ = db.mark_failed(&id, &error.to_string());
        return Err(error);
    }
    if let Err(error) = db.mark_active(&id) {
        bail!("remote publication may have been created, but local state update failed: {error:#}; keep the local database backup for revoke");
    }
    println!("{}", publication_url(&endpoint, &id, &viewer_key));
    output::detail("publication", &id);
    output::detail("expires", draft.snapshot.expires_at());
    Ok(())
}

fn list(args: PublishListArgs) -> Result<()> {
    let mut db = PublicationDb::open()?;
    db.refresh_expired()?;
    let rows = db.rows()?;
    let items = rows.iter().map(list_item).collect::<Vec<_>>();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        if items.is_empty() {
            println!("暂无公开链接");
        } else {
            for item in items {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    item.publication_id, item.status, item.title, item.provider, item.expires_at
                );
            }
        }
    }
    Ok(())
}

fn link(args: PublishIdArgs) -> Result<()> {
    let db = PublicationDb::open()?;
    let row = db
        .find(&args.publication_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown publication id `{}`", args.publication_id))?;
    if row.status != PublicationStatus::Active {
        bail!(
            "publication `{}` is {} and has no usable link",
            row.id,
            row.status.as_str()
        );
    }
    if is_expired(&row.expires_at) {
        bail!("publication `{}` has expired", row.id);
    }
    println!(
        "{}",
        publication_url(&row.endpoint, &row.id, &row.viewer_key)
    );
    Ok(())
}

fn revoke(args: PublishRevokeArgs) -> Result<()> {
    let db = PublicationDb::open()?;
    let row = db
        .find(&args.publication_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown publication id `{}`", args.publication_id))?;
    if row.status == PublicationStatus::Revoked {
        return Ok(());
    }
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            bail!("non-interactive revoke requires --yes");
        }
        if !dialoguer::Confirm::new()
            .with_prompt(format!("撤销 {}？", row.id))
            .default(false)
            .interact()?
        {
            bail!("revoke cancelled");
        }
    }
    match delete_remote(&row.endpoint, &row.id, &row.management_token) {
        Ok(()) => {
            db.mark_revoked(&row.id)?;
            output::success(format!("revoked {}", row.id));
            Ok(())
        }
        Err(error) => {
            let _ = db.record_error(&row.id, &error.to_string());
            Err(error)
        }
    }
}

fn format_human_preview_meta(draft: &PublicationDraft) -> String {
    let mut out = format!(
        "标题: {}\nProvider: {}\nSchema: v{}\n轮次数: {}\n消息数: {}\n预计过期: {}\n内容 SHA-256: {}\n自动脱敏: {} 项\n",
        draft.snapshot.title(),
        draft.snapshot.provider(),
        draft.snapshot.schema_version(),
        draft.turn_count(),
        draft.item_count(),
        draft.snapshot.expires_at(),
        draft.content_sha256,
        draft.redaction_count,
    );
    if draft.risks.is_empty() {
        out.push_str("风险提示: 无\n");
    } else {
        out.push_str("风险提示:\n");
        for risk in &draft.risks {
            out.push_str(&format!(
                "  - {}: {} 项{}\n",
                risk.kind,
                risk.count,
                format_item_indices(&risk.item_indices)
            ));
        }
    }
    out
}

fn format_create_summary(
    draft: &PublicationDraft,
    expiry: &str,
    envelope_size: usize,
) -> Vec<(&'static str, String)> {
    vec![
        ("title", draft.snapshot.title().to_string()),
        ("turns", draft.turn_count().to_string()),
        ("messages", draft.item_count().to_string()),
        ("schema", format!("v{}", draft.snapshot.schema_version())),
        ("envelope", format!("{envelope_size} bytes")),
        ("redactions", draft.redaction_count.to_string()),
        ("expiry", expiry.to_string()),
        (
            "source",
            "local WorkSet; original refs and paths stay local".to_string(),
        ),
    ]
}

fn print_create_summary(draft: &PublicationDraft, expiry: &str, envelope_size: usize) {
    for (label, value) in format_create_summary(draft, expiry, envelope_size) {
        output::detail(label, value);
    }
}

fn print_risks(draft: &PublicationDraft) {
    if draft.risks.is_empty() {
        eprintln!("risk warnings: none");
    } else {
        for risk in &draft.risks {
            eprintln!(
                "risk {}: {} item(s){}",
                risk.kind,
                risk.count,
                format_item_indices(&risk.item_indices)
            );
        }
    }
}

fn format_item_indices(indices: &[usize]) -> String {
    if indices.is_empty() {
        String::new()
    } else {
        format!(
            " (message {})",
            indices
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn is_warning_only(kind: &str) -> bool {
    matches!(kind, "absolute_path" | "email" | "internal_url")
}

fn require_allow_warnings(has_warnings: bool, allow_warnings: bool) -> Result<()> {
    if has_warnings && !allow_warnings {
        bail!("publish with privacy warnings requires --allow-warnings");
    }
    Ok(())
}

fn resolve_endpoint(config: &SivtrConfig) -> Result<String> {
    let endpoint = config.publish.endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        bail!(
            "[publish].endpoint is not set; add the publication service URL to config.toml (for example https://share.hnnulwh.cn)"
        );
    }
    Ok(endpoint.to_string())
}

fn random_token(length: usize) -> Result<String> {
    let mut bytes = vec![0_u8; length];
    getrandom::fill(&mut bytes).context("OS random source unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn compress_snapshot(draft: &PublicationDraft) -> Result<Vec<u8>> {
    ensure_snapshot_plaintext_limit(draft.canonical_json.len())?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(draft.canonical_json.as_bytes())?;
    Ok(encoder.finish()?)
}

fn ensure_snapshot_plaintext_limit(len: usize) -> Result<()> {
    if len > SNAPSHOT_PLAINTEXT_LIMIT {
        bail!(
            "publication snapshot is {len} bytes uncompressed; maximum is 16 MiB; narrow the WorkSet"
        );
    }
    Ok(())
}

fn encrypt_snapshot(draft: &PublicationDraft, id: &str, viewer_key: &str) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0_u8; 12];
    getrandom::fill(&mut nonce_bytes).context("OS random source unavailable")?;
    encrypt_snapshot_with_nonce(draft, id, viewer_key, nonce_bytes)
}

fn encrypt_snapshot_with_nonce(
    draft: &PublicationDraft,
    id: &str,
    viewer_key: &str,
    nonce_bytes: [u8; 12],
) -> Result<Vec<u8>> {
    let key_bytes = URL_SAFE_NO_PAD
        .decode(viewer_key)
        .context("invalid generated viewer key")?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut compressed = compress_snapshot(draft)?;
    let aad = format!("sivtr-publication-v1:{id}");
    let tag = cipher
        .encrypt_in_place_detached(nonce, aad.as_bytes(), &mut compressed)
        .map_err(|_| anyhow::anyhow!("AES-GCM encryption failed"))?;
    let mut envelope = Vec::with_capacity(8 + 2 + 12 + compressed.len() + tag.len());
    envelope.extend_from_slice(ENVELOPE_MAGIC);
    envelope.extend_from_slice(&[1, 1]); // envelope v1, gzip compression
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&compressed);
    envelope.extend_from_slice(&tag);
    if envelope.len() > ENVELOPE_LIMIT {
        bail!("encrypted publication envelope exceeds 5 MiB");
    }
    Ok(envelope)
}

fn publication_url(endpoint: &str, id: &str, viewer_key: &str) -> String {
    format!(
        "{}/s/{}#k={}",
        endpoint.trim_end_matches('/'),
        id,
        viewer_key
    )
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

fn upload(endpoint: &str, id: &str, management_token: &str, envelope: &[u8]) -> Result<()> {
    let url = format!("{endpoint}/api/v1/publications/{id}");
    let response = agent(Duration::from_secs(30))
        .put(&url)
        .header("Content-Type", "application/octet-stream")
        .header("X-Sivtr-Management-Token", management_token)
        .send(envelope)
        .with_context(|| format!("publication upload failed: {url}"))?;
    if !response.status().is_success() {
        bail!("publication upload returned HTTP {}", response.status());
    }
    Ok(())
}

fn delete_remote(endpoint: &str, id: &str, management_token: &str) -> Result<()> {
    let url = format!("{endpoint}/api/v1/publications/{id}");
    let response = agent(Duration::from_secs(30))
        .delete(&url)
        .header("X-Sivtr-Management-Token", management_token)
        .call()
        .with_context(|| format!("publication revoke failed: {url}"))?;
    if !response.status().is_success() {
        bail!("publication revoke returned HTTP {}", response.status());
    }
    Ok(())
}

fn list_item(row: &PublicationRow) -> PublicationListItem {
    PublicationListItem {
        publication_id: row.id.clone(),
        title: row.title.clone(),
        provider: row.provider.clone(),
        status: row.status.as_str().to_string(),
        created_at: row.created_at.clone(),
        expires_at: row.expires_at.clone(),
        redaction_count: row.redaction_count,
        warning_count: row.warning_count,
        content_sha256: row.content_sha256.clone(),
        last_error: row.last_error.clone(),
    }
}

struct PublicationDb {
    connection: Connection,
}

impl PublicationDb {
    fn open() -> Result<Self> {
        let dir = workspace::data_dir();
        std::fs::create_dir_all(&dir)?;
        restrict_directory(&dir)?;
        let path = dir.join("publication-state.db");
        let connection = Connection::open(&path)?;
        restrict_file(&path)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS publications (
                publication_id TEXT PRIMARY KEY,
                endpoint TEXT NOT NULL,
                viewer_key TEXT NOT NULL,
                management_token TEXT NOT NULL,
                title TEXT NOT NULL,
                provider TEXT NOT NULL,
                source_refs TEXT NOT NULL,
                content_sha256 TEXT NOT NULL,
                redaction_count INTEGER NOT NULL,
                warning_count INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                status TEXT NOT NULL,
                last_error TEXT
            );",
        )?;
        Ok(Self { connection })
    }

    fn insert_pending(&mut self, row: &PublicationRow) -> Result<()> {
        self.connection.execute(
            "INSERT INTO publications (publication_id, endpoint, viewer_key, management_token, title, provider, source_refs, content_sha256, redaction_count, warning_count, created_at, expires_at, status, last_error) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![row.id, row.endpoint, row.viewer_key, row.management_token, row.title, row.provider, row.source_refs, row.content_sha256, row.redaction_count, row.warning_count, row.created_at, row.expires_at, row.status.as_str(), row.last_error],
        )?;
        Ok(())
    }

    fn update_status(
        &self,
        id: &str,
        status: PublicationStatus,
        error: Option<&str>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE publications SET status = ?1, last_error = ?2 WHERE publication_id = ?3",
            params![status.as_str(), error, id],
        )?;
        Ok(())
    }

    fn mark_active(&self, id: &str) -> Result<()> {
        self.update_status(id, PublicationStatus::Active, None)
    }
    fn mark_failed(&self, id: &str, error: &str) -> Result<()> {
        self.update_status(id, PublicationStatus::Failed, Some(error))
    }
    fn mark_revoked(&self, id: &str) -> Result<()> {
        self.update_status(id, PublicationStatus::Revoked, None)
    }
    fn record_error(&self, id: &str, error: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE publications SET last_error = ?1 WHERE publication_id = ?2",
            params![error, id],
        )?;
        Ok(())
    }

    fn find(&self, id: &str) -> Result<Option<PublicationRow>> {
        self.connection.query_row("SELECT publication_id, endpoint, viewer_key, management_token, title, provider, source_refs, content_sha256, redaction_count, warning_count, created_at, expires_at, status, last_error FROM publications WHERE publication_id = ?1", params![id], row_from_query).optional().map_err(Into::into)
    }

    fn rows(&self) -> Result<Vec<PublicationRow>> {
        let mut statement = self.connection.prepare("SELECT publication_id, endpoint, viewer_key, management_token, title, provider, source_refs, content_sha256, redaction_count, warning_count, created_at, expires_at, status, last_error FROM publications ORDER BY created_at DESC")?;
        let rows = statement
            .query_map([], row_from_query)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn refresh_expired(&mut self) -> Result<()> {
        let now = Utc::now();
        let rows = self
            .connection
            .prepare("SELECT publication_id, expires_at FROM publications WHERE status IN ('pending', 'active')")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, expires_at) in rows {
            if is_expired_at(&expires_at, now) {
                self.connection.execute(
                    "UPDATE publications SET status = 'expired' WHERE publication_id = ?1",
                    params![id],
                )?;
            }
        }
        Ok(())
    }
}

fn is_expired(value: &str) -> bool {
    is_expired_at(value, Utc::now())
}

fn is_expired_at(value: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc) <= now)
        .unwrap_or(true)
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn row_from_query(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicationRow> {
    Ok(PublicationRow {
        id: row.get(0)?,
        endpoint: row.get(1)?,
        viewer_key: row.get(2)?,
        management_token: row.get(3)?,
        title: row.get(4)?,
        provider: row.get(5)?,
        source_refs: row.get(6)?,
        content_sha256: row.get(7)?,
        redaction_count: row.get(8)?,
        warning_count: row.get(9)?,
        created_at: row.get(10)?,
        expires_at: row.get(11)?,
        status: row
            .get::<_, String>(12)?
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        last_error: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_has_publication_header_and_aad_is_id_bound() {
        let snapshot = sivtr_core::publication::PublicConversationV1 {
            schema_version: 1,
            title: "t".into(),
            provider: "codex".into(),
            published_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2026-01-08T00:00:00Z".into(),
            items: vec![],
        };
        let draft = PublicationDraft {
            canonical_json: serde_json::to_string(&snapshot).unwrap(),
            snapshot: PublicConversationSnapshot::V1(snapshot),
            content_sha256: "x".into(),
            redaction_count: 0,
            risks: vec![],
            source_provider: "codex".into(),
            source_refs: vec![],
        };
        let key = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let envelope = encrypt_snapshot(&draft, "7d_abc", &key).unwrap();
        assert_eq!(&envelope[..8], ENVELOPE_MAGIC);
        assert_eq!(envelope[8], 1);
        assert_eq!(envelope[9], 1);
        assert_ne!(encrypt_snapshot(&draft, "7d_abc", &key).unwrap(), envelope);
        let fixture = encrypt_snapshot_with_nonce(
            &draft,
            "7d_0123456789abcdefghijkl",
            &key,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        )
        .unwrap();
        assert_eq!(fixture.len(), 160);
    }

    #[test]
    fn url_keeps_key_in_fragment() {
        let url = publication_url(
            "https://share.hnnulwh.cn",
            "7d_0123456789abcdefghijkl",
            "key",
        );
        assert_eq!(
            url,
            "https://share.hnnulwh.cn/s/7d_0123456789abcdefghijkl#k=key"
        );
    }

    #[test]
    fn local_registry_tracks_pending_active_failed_and_revoked() {
        let connection = Connection::open_in_memory().unwrap();
        let mut db = PublicationDb::from_connection(connection).unwrap();
        let row = PublicationRow {
            id: "7d_0123456789abcdefghijkl".into(),
            endpoint: "https://share.hnnulwh.cn".into(),
            viewer_key: "k".into(),
            management_token: "m".into(),
            title: "title".into(),
            provider: "codex".into(),
            source_refs: "[]".into(),
            content_sha256: "hash".into(),
            redaction_count: 0,
            warning_count: 0,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            status: PublicationStatus::Pending,
            last_error: None,
        };
        db.insert_pending(&row).unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Pending
        );
        db.mark_active(&row.id).unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Active
        );
        db.mark_failed(&row.id, "network").unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Failed
        );
        db.mark_revoked(&row.id).unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Revoked
        );
    }

    #[test]
    fn uncompressed_snapshot_over_16mib_is_rejected() {
        assert!(ensure_snapshot_plaintext_limit(SNAPSHOT_PLAINTEXT_LIMIT).is_ok());
        assert!(ensure_snapshot_plaintext_limit(SNAPSHOT_PLAINTEXT_LIMIT + 1).is_err());
    }

    #[test]
    fn warnings_always_require_explicit_allow() {
        assert!(require_allow_warnings(true, false).is_err());
        assert!(require_allow_warnings(true, true).is_ok());
        assert!(require_allow_warnings(false, false).is_ok());
    }

    #[test]
    fn empty_endpoint_is_rejected() {
        let mut config = SivtrConfig::default();
        config.publish.endpoint.clear();
        assert!(resolve_endpoint(&config).is_err());
        config.publish.endpoint = "https://share.hnnulwh.cn/".into();
        assert_eq!(
            resolve_endpoint(&config).unwrap(),
            "https://share.hnnulwh.cn"
        );
    }

    fn chat_turn(session: &str, index: usize, thinking: &str, tool_out: &str) -> WorkRecord {
        WorkRecord {
            schema_version: 3,
            work_ref: WorkRef::agent(sivtr_core::ai::AgentProvider::Codex, session, index),
            kind: sivtr_core::record::WorkRecordKind::ChatTurn,
            source: sivtr_core::record::WorkSource {
                channel: sivtr_core::record::WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: sivtr_core::record::WorkSessionRef {
                id: session.into(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime::default(),
            status: None,
            title: "Demo".into(),
            parts: vec![
                sivtr_core::record::WorkPart {
                    seq: 1,
                    occurred_at: None,
                    data: sivtr_core::record::WorkPartData::User {
                        content: "question".into(),
                    },
                },
                sivtr_core::record::WorkPart {
                    seq: 2,
                    occurred_at: None,
                    data: sivtr_core::record::WorkPartData::ToolCall {
                        call_id: Some("c1".into()),
                        tool: Some("Bash".into()),
                        input: serde_json::json!({"command": "ls"}),
                    },
                },
                sivtr_core::record::WorkPart {
                    seq: 3,
                    occurred_at: None,
                    data: sivtr_core::record::WorkPartData::ToolResult {
                        call_id: Some("c1".into()),
                        tool: Some("Bash".into()),
                        output: serde_json::json!({"stdout": tool_out}),
                        start_line: None,
                    },
                },
                sivtr_core::record::WorkPart {
                    seq: 4,
                    occurred_at: None,
                    data: sivtr_core::record::WorkPartData::Thinking {
                        content: thinking.into(),
                    },
                },
                sivtr_core::record::WorkPart {
                    seq: 5,
                    occurred_at: None,
                    data: sivtr_core::record::WorkPartData::Assistant {
                        content: "reply".into(),
                    },
                },
            ],
        }
    }

    fn seqs(anchors: &[WorkRef]) -> Vec<usize> {
        anchors
            .iter()
            .map(|anchor| anchor.part().expect("part anchor"))
            .collect()
    }

    #[test]
    fn expand_picker_anchors_scopes_whole_part_and_half_refs() {
        let record = chat_turn("session", 1, "thinking", "ok");
        let whole =
            expand_picker_anchors(std::slice::from_ref(&record), &[record.work_ref.whole()])
                .unwrap();
        assert_eq!(seqs(&whole), vec![1, 2, 3, 4, 5]);

        let tool_pair = expand_picker_anchors(
            std::slice::from_ref(&record),
            &[record.work_ref.with_part(2)],
        )
        .unwrap();
        assert_eq!(seqs(&tool_pair), vec![2, 3]);

        let input_half = expand_picker_anchors(
            std::slice::from_ref(&record),
            &[record.work_ref.with_part(1)],
        )
        .unwrap();
        assert_eq!(seqs(&input_half), vec![1]);

        let other = chat_turn("other", 1, "x", "y");
        assert!(
            expand_picker_anchors(std::slice::from_ref(&record), &[other.work_ref.whole()])
                .is_err()
        );
    }

    #[test]
    fn pick_saved_workset_drops_unselected_turn_bodies() {
        let first = chat_turn("session", 1, "SECRET_TURN1", "tool-1");
        let selected = chat_turn("session", 2, "keep-thinking", "keep-tool");
        let last = chat_turn("session", 3, "secret-think", "SECRET_TURN3");
        let anchors = vec![
            selected.work_ref.with_part(1),
            selected.work_ref.with_part(5),
        ];
        let set = workset::WorkSet::with_anchors(
            ".",
            vec![first, selected.clone(), last],
            vec![selected.work_ref.whole()],
        );
        let slim = publication_workset(set, anchors.clone());
        assert_eq!(slim.records.len(), 1);
        assert_eq!(slim.records[0].work_ref.index(), 2);
        let persisted = serde_json::to_string(&slim.records).unwrap();
        assert!(!persisted.contains("SECRET_TURN1"));
        assert!(!persisted.contains("SECRET_TURN3"));
        assert!(persisted.contains("keep-thinking"));
        assert!(persisted.contains("keep-tool"));

        let policy = PublicationPolicy {
            published_at: Some(
                DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..PublicationPolicy::default()
        };
        let full = create_publication_draft(
            &[
                chat_turn("session", 1, "SECRET_TURN1", "tool-1"),
                selected.clone(),
                chat_turn("session", 3, "secret-think", "SECRET_TURN3"),
            ],
            &anchors,
            &policy,
        )
        .unwrap();
        let from_saved = create_publication_draft(&slim.records, &slim.anchors, &policy).unwrap();
        assert_eq!(full.content_sha256, from_saved.content_sha256);
        let PublicConversationSnapshot::V2(snapshot) = &from_saved.snapshot else {
            panic!("pick-saved WorkSet is schema v2");
        };
        assert!(snapshot.items[0].gap_before);
    }

    #[test]
    fn human_preview_and_create_summary_name_schema_version() {
        let v1_record = chat_turn("session", 1, "thinking", "ok");
        let v1 = create_publication_draft(
            std::slice::from_ref(&v1_record),
            &[],
            &PublicationPolicy::default(),
        )
        .unwrap();
        let v2 = create_publication_draft(
            std::slice::from_ref(&v1_record),
            &[
                v1_record.work_ref.with_part(1),
                v1_record.work_ref.with_part(5),
            ],
            &PublicationPolicy::default(),
        )
        .unwrap();

        let v1_preview = format_human_preview_meta(&v1);
        let v2_preview = format_human_preview_meta(&v2);
        assert!(
            v1_preview.contains("Schema: v1"),
            "v1 preview missing schema: {v1_preview}"
        );
        assert!(
            v2_preview.contains("Schema: v2"),
            "v2 preview missing schema: {v2_preview}"
        );

        let v1_summary = format_create_summary(&v1, "7d", 12);
        let v2_summary = format_create_summary(&v2, "7d", 12);
        assert!(v1_summary
            .iter()
            .any(|(label, value)| *label == "schema" && value == "v1"));
        assert!(v2_summary
            .iter()
            .any(|(label, value)| *label == "schema" && value == "v2"));
    }
}
