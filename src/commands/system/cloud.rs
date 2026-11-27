//! Cloud sync and capability sharing command handlers.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use object_store::{
    aws::AmazonS3Builder, path::Path as ObjectPath, ObjectStore, ObjectStoreExt, PutMode,
    PutOptions,
};
use sivtr_core::archive;
use sivtr_core::cloud::{self, CapabilityScope, CloudBackend, CloudSettings, LocalDirBackend};
use sivtr_core::workspace;

use crate::cli::{
    ShareAction, ShareAttachArgs, ShareCommand, ShareCreateArgs, ShareRegisterArgs,
    ShareRevokeArgs, SyncAction, SyncCommand, SyncInitArgs, SyncRemoteArgs,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AttachedCapability {
    remote: String,
    scope: String,
    sessions: Vec<(String, String)>,
}

pub fn execute_sync(command: &SyncCommand) -> Result<()> {
    match &command.action {
        SyncAction::Init(args) => sync_init(args),
        SyncAction::Now(args) => sync_now(args),
        SyncAction::Status(args) => sync_status(args),
    }
}

pub fn execute_share(command: &ShareCommand) -> Result<()> {
    match &command.action {
        ShareAction::Register(args) => share_register(args),
        ShareAction::Create(args) => share_create(args),
        ShareAction::Attach(args) => share_attach(args),
        ShareAction::Revoke(args) => share_revoke(args),
    }
}

fn sync_init(args: &SyncInitArgs) -> Result<()> {
    ensure!(
        cloud::load_settings()?.is_none(),
        "cloud sync is already initialized; remove {}",
        cloud::settings_path().display()
    );
    let remote = normalize_remote(&args.remote)?;
    let backend = backend_for_remote(&remote)?;
    let key_file = args
        .key_file
        .clone()
        .unwrap_or_else(|| workspace::home_dir().join("cloud.key"));
    cloud::create_library_key(&key_file)?;
    let recipient_key = recipient_key_path(None);
    if !recipient_key.exists() {
        cloud::create_recipient_key(&recipient_key)?;
    }
    cloud::register_public_key(
        backend.as_ref(),
        &device_id()?,
        &cloud::recipient_public_key(&cloud::load_recipient_key(&recipient_key)?),
    )?;
    let settings = CloudSettings { remote, key_file };
    cloud::save_settings(&settings)?;
    if args.json {
        print_json(&settings)
    } else {
        println!("cloud sync initialized");
        println!("remote: {}", settings.remote);
        println!("key file: {}", settings.key_file.display());
        Ok(())
    }
}

fn sync_now(args: &SyncRemoteArgs) -> Result<()> {
    let mut settings = load_settings(args.remote.as_deref())?;
    if let Some(key_file) = args.key_file.as_deref() {
        settings.key_file = key_file.to_path_buf();
    }
    let backend = backend_for_remote(&settings.remote)?;
    let key = cloud::load_library_key(&settings.key_file)?;
    let mut conn = archive::open()?;
    let attached = load_attached_sessions()?;
    let pushed =
        cloud::sync_push_excluding(&conn, backend.as_ref(), &key, &device_id()?, &attached)?;
    let pulled = cloud::sync_pull(&conn, backend.as_ref(), &key)?;
    let reclaimed = if args.prune {
        Some(cloud::reclaim_cache(
            &mut conn,
            backend.as_ref(),
            &key,
            &CapabilityScope::All,
        )?)
    } else {
        None
    };
    if args.json {
        print_json(&serde_json::json!({
            "pushed": pushed,
            "pulled": pulled,
            "reclaimed": reclaimed,
        }))
    } else {
        println!(
            "sync generation {}: {} pushed, {} added, {} unchanged",
            pushed.generation,
            pushed.sessions_written,
            pulled.sessions_added,
            pulled.sessions_unchanged
        );
        if let Some(count) = reclaimed {
            println!("cache reclaimed: {count} sessions");
        }
        for conflict in &pulled.conflicts {
            println!("conflict: {conflict}");
        }
        Ok(())
    }
}

fn sync_status(args: &SyncRemoteArgs) -> Result<()> {
    let mut settings = load_settings(args.remote.as_deref())?;
    if let Some(key_file) = args.key_file.as_deref() {
        settings.key_file = key_file.to_path_buf();
    }
    let backend = backend_for_remote(&settings.remote)?;
    let key = cloud::load_library_key(&settings.key_file)?;
    let conn = archive::open()?;
    let report = cloud::sync_status(&conn, backend.as_ref(), &key)?;
    if args.json {
        print_json(&report)
    } else {
        println!("remote generation: {}", report.generation);
        println!(
            "sessions: {} local, {} remote",
            report.local_sessions, report.remote_sessions
        );
        for missing in &report.missing {
            println!("missing: {missing}");
        }
        for changed in &report.changed {
            println!("changed: {changed}");
        }
        Ok(())
    }
}

fn resolve_share_settings(
    remote_override: Option<&str>,
    missing_message: &str,
) -> Result<CloudSettings> {
    let mut settings = cloud::load_settings()?.unwrap_or(CloudSettings {
        remote: String::new(),
        key_file: PathBuf::new(),
    });
    if let Some(remote) = remote_override {
        settings.remote = remote.to_string();
    }
    ensure!(!settings.remote.is_empty(), "{missing_message}");
    Ok(settings)
}

fn share_register(args: &ShareRegisterArgs) -> Result<()> {
    let settings = resolve_share_settings(
        args.remote.as_deref(),
        "fresh home requires --remote when registering a device",
    )?;
    let backend = backend_for_remote(&settings.remote)?;
    let path = recipient_key_path(args.key_file.as_deref());
    let secret = if path.exists() {
        cloud::load_recipient_key(&path)?
    } else {
        cloud::create_recipient_key(&path)?
    };
    cloud::register_public_key(
        backend.as_ref(),
        &args.device,
        &cloud::recipient_public_key(&secret),
    )?;
    if args.json {
        print_json(&serde_json::json!({ "device": args.device }))
    } else {
        println!("registered device: {}", args.device);
        Ok(())
    }
}

fn share_create(args: &ShareCreateArgs) -> Result<()> {
    let settings = load_settings(None)?;
    let backend = backend_for_remote(&settings.remote)?;
    let library_key = cloud::load_library_key(&settings.key_file)?;
    let scope = parse_scope(&args.scope)?;
    let recipient = cloud::load_public_key(backend.as_ref(), &args.to)?;
    let id = args
        .id
        .clone()
        .unwrap_or_else(|| format!("cap-{}", random_token(12)));
    cloud::create_capability(backend.as_ref(), &library_key, &recipient, scope, &id)?;
    if args.json {
        print_json(&serde_json::json!({ "capability_id": id }))
    } else {
        println!("capability created: {id}");
        Ok(())
    }
}

fn share_attach(args: &ShareAttachArgs) -> Result<()> {
    let settings = resolve_share_settings(
        args.remote.as_deref(),
        "fresh home requires --remote when attaching a capability",
    )?;
    let backend = backend_for_remote(&settings.remote)?;
    let capability = cloud::read_capability(backend.as_ref(), &args.capability_id)?
        .with_context(|| format!("capability `{}` is unavailable", args.capability_id))?;
    let recipient_path = recipient_key_path(args.key_file.as_deref());
    ensure!(
        recipient_path.exists(),
        "recipient key is missing; run `sivtr share register` first"
    );
    let payload =
        cloud::unwrap_capability(&capability, &cloud::load_recipient_key(&recipient_path)?)?;
    let attached_session_ids: Vec<(String, String)> = payload
        .sessions
        .iter()
        .map(|scoped| {
            (
                scoped.entry.provider.clone(),
                scoped.entry.session_id.clone(),
            )
        })
        .collect();
    record_attached_sessions(&attached_session_ids)?;
    let conn = archive::open()?;
    let report =
        cloud::sync_pull_capability(&conn, backend.as_ref(), &args.capability_id, &payload)?;
    let attached = AttachedCapability {
        remote: settings.remote.clone(),
        scope: scope_label(&capability.scope),
        sessions: payload
            .sessions
            .iter()
            .map(|scoped| {
                (
                    scoped.entry.provider.clone(),
                    scoped.entry.session_id.clone(),
                )
            })
            .collect(),
    };
    let state_path = attached_state_path(&args.capability_id);
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create `{}`", parent.display()))?;
    }
    std::fs::write(&state_path, toml::to_string(&attached)?)
        .with_context(|| format!("Failed to write `{}`", state_path.display()))?;
    if args.json {
        print_json(&report)
    } else {
        println!(
            "attached {}: {} sessions added, {} unchanged",
            args.capability_id, report.sessions_added, report.sessions_unchanged
        );
        for conflict in &report.conflicts {
            println!("conflict: {conflict}");
        }
        Ok(())
    }
}

fn share_revoke(args: &ShareRevokeArgs) -> Result<()> {
    let remote = if let Some(remote) = args.remote.as_deref() {
        remote.to_string()
    } else if attached_state_path(&args.capability_id).exists() {
        load_attached_capability(&args.capability_id)?.remote
    } else {
        cloud::load_settings()?
            .context("cloud sync is not initialized and no attachment record exists")?
            .remote
    };
    let backend = backend_for_remote(&remote)?;
    cloud::revoke_capability(backend.as_ref(), &args.capability_id)?;
    let _ = std::fs::remove_file(attached_state_path(&args.capability_id));
    if args.json {
        print_json(&serde_json::json!({ "revoked": args.capability_id }))
    } else {
        println!("revoked capability: {}", args.capability_id);
        Ok(())
    }
}

fn load_settings(remote_override: Option<&str>) -> Result<CloudSettings> {
    let mut settings =
        cloud::load_settings()?.context("cloud sync is not initialized; run `sivtr sync init`")?;
    if let Some(remote) = remote_override {
        settings.remote = remote.to_string();
    }
    Ok(settings)
}

fn recipient_key_path(override_path: Option<&Path>) -> PathBuf {
    override_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace::home_dir().join("recipient.key"))
}

fn attached_state_path(id: &str) -> PathBuf {
    workspace::home_dir()
        .join("caps")
        .join(format!("{id}.toml"))
}

fn attached_sessions_path() -> PathBuf {
    workspace::home_dir().join("attached-sessions.json")
}

fn load_attached_capability(id: &str) -> Result<AttachedCapability> {
    let path = attached_state_path(id);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("capability `{}` is not attached", id))?;
    toml::from_str(&text).context("Failed to read attached capability")
}

fn load_attached_sessions() -> Result<std::collections::BTreeSet<(String, String)>> {
    let path = attached_sessions_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(std::collections::BTreeSet::new());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()));
        }
    };
    serde_json::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))
}

fn record_attached_sessions(sessions: &[(String, String)]) -> Result<()> {
    let path = attached_sessions_path();
    let mut known = load_attached_sessions()?;
    known.extend(sessions.iter().cloned());
    let entries: Vec<serde_json::Value> = known
        .iter()
        .map(|(provider, session_id)| serde_json::json!([provider, session_id]))
        .collect();
    std::fs::write(&path, serde_json::to_string_pretty(&entries)?)
        .with_context(|| format!("Failed to write {}", path.display()))
}

fn parse_scope(value: &str) -> Result<CapabilityScope> {
    match value {
        "all" => Ok(CapabilityScope::All),
        value if value.starts_with("session:") => {
            let id = value
                .strip_prefix("session:")
                .expect("checked by starts_with");
            ensure!(!id.is_empty(), "session scope requires an id");
            Ok(CapabilityScope::Session(id.to_string()))
        }
        value if value.starts_with("provider:") => {
            let provider = value
                .strip_prefix("provider:")
                .expect("checked by starts_with");
            ensure!(!provider.is_empty(), "provider scope requires a name");
            Ok(CapabilityScope::Provider(provider.to_string()))
        }
        _ => bail!("scope must be all, session:<id>, or provider:<name>"),
    }
}

fn scope_label(scope: &CapabilityScope) -> String {
    match scope {
        CapabilityScope::All => "all".to_string(),
        CapabilityScope::Session(id) => format!("session:{id}"),
        CapabilityScope::Provider(provider) => format!("provider:{provider}"),
    }
}

fn device_id() -> Result<String> {
    let name = env::var("SIVTR_DEVICE_NAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "sivtr-device".to_string());
    let normalized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let normalized = normalized.trim_matches('-');
    ensure!(!normalized.is_empty(), "device name is empty");
    Ok(normalized.to_string())
}

fn random_token(bytes: usize) -> String {
    let mut data = vec![0_u8; bytes];
    getrandom::fill(&mut data).expect("OS random source unavailable");
    URL_SAFE_NO_PAD.encode(data)
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn normalize_remote(remote: &str) -> Result<String> {
    if let Some(path) = remote.strip_prefix("file://") {
        let path = PathBuf::from(path);
        let absolute = if path.is_absolute() {
            path
        } else {
            env::current_dir()
                .context("Failed to resolve current directory")?
                .join(path)
        };
        return Ok(format!(
            "file://{}",
            absolute.to_string_lossy().replace('\\', "/")
        ));
    }
    Ok(remote.to_string())
}

fn backend_for_remote(remote: &str) -> Result<Box<dyn CloudBackend>> {
    if let Some(path) = remote.strip_prefix("file://") {
        ensure!(!path.is_empty(), "file:// remote requires a path");
        return Ok(Box::new(LocalDirBackend::new(PathBuf::from(path))));
    }
    if let Some(rest) = remote.strip_prefix("s3://") {
        let (bucket, prefix) = match rest.split_once('/') {
            Some((bucket, prefix)) => (bucket, prefix),
            None => (rest, ""),
        };
        ensure!(!bucket.is_empty(), "s3:// remote requires a bucket");
        let region = env::var("SIVTR_S3_REGION")
            .or_else(|_| env::var("AWS_REGION"))
            .unwrap_or_else(|_| "auto".to_string());
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .with_region(region);
        if let Ok(endpoint) =
            env::var("SIVTR_S3_ENDPOINT").or_else(|_| env::var("AWS_ENDPOINT_URL"))
        {
            builder = builder.with_endpoint(endpoint).with_allow_http(true);
        }
        let store = builder
            .build()
            .context("Failed to configure S3-compatible cloud backend")?;
        return Ok(Box::new(S3Backend {
            store,
            prefix: prefix.trim_matches('/').to_string(),
        }));
    }
    bail!("unsupported cloud remote `{remote}`; use file:// or s3://");
}

struct S3Backend {
    store: object_store::aws::AmazonS3,
    prefix: String,
}

impl S3Backend {
    fn path(&self, key: &str) -> ObjectPath {
        if self.prefix.is_empty() {
            ObjectPath::from(key)
        } else {
            ObjectPath::from(format!("{}/{}", self.prefix, key))
        }
    }
}

impl CloudBackend for S3Backend {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            self.store
                .put(&self.path(key), bytes.to_vec().into())
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from)
        })
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<bool> {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let options = PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            };
            match self
                .store
                .put_opts(&self.path(key), bytes.to_vec().into(), options)
                .await
            {
                Ok(_) => Ok(true),
                Err(object_store::Error::AlreadyExists { .. }) => Ok(false),
                Err(error) => Err(anyhow::Error::from(error)),
            }
        })
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            match self.store.get(&self.path(key)).await {
                Ok(result) => Ok(Some(result.bytes().await?.as_ref().to_vec())),
                Err(object_store::Error::NotFound { .. }) => Ok(None),
                Err(error) => Err(anyhow::Error::from(error)),
            }
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            self.store
                .delete(&self.path(key))
                .await
                .map_err(anyhow::Error::from)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scopes() {
        assert_eq!(parse_scope("all").unwrap(), CapabilityScope::All);
        assert_eq!(
            parse_scope("session:abc").unwrap(),
            CapabilityScope::Session("abc".into())
        );
        assert!(parse_scope("bad").is_err());
    }

    #[test]
    fn rejects_unknown_backend() {
        assert!(backend_for_remote("gs://bucket").is_err());
    }
}
