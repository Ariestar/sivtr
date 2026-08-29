//! Publication envelope encoding and remote transport.

use aes_gcm::{
    aead::{AeadInOut, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use flate2::{write::GzEncoder, Compression};
use sivtr_core::{config::SivtrConfig, publication::PublicationDraft};
use std::io::Write;
use std::time::Duration;

pub(super) const ENVELOPE_LIMIT: usize = 5 * 1024 * 1024;
pub(super) const SNAPSHOT_PLAINTEXT_LIMIT: usize = 16 * 1024 * 1024;
pub(super) const ENVELOPE_MAGIC: &[u8; 8] = b"SIVTPUB1";

pub(super) fn publication_envelope_size(compressed: &[u8]) -> Result<usize> {
    let envelope_size = compressed
        .len()
        .checked_add(8 + 2 + 12 + 16)
        .ok_or_else(|| anyhow::anyhow!("encrypted publication envelope size overflow"))?;
    if envelope_size > ENVELOPE_LIMIT {
        bail!(
            "encrypted publication envelope is {} bytes; maximum is 5 MiB; narrow the WorkSet",
            envelope_size
        );
    }
    Ok(envelope_size)
}

pub(super) fn resolve_endpoint(config: &SivtrConfig) -> Result<String> {
    let endpoint = config.publish.endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        bail!(
            "[publish].endpoint is not set; add the publication service URL to config.toml (for example https://share.hnnulwh.cn)"
        );
    }
    let Some((scheme, authority)) = endpoint.split_once("://") else {
        bail!("[publish].endpoint must include an https:// scheme");
    };
    let authority = authority.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        bail!("[publish].endpoint is missing a host");
    }
    let secure = scheme.eq_ignore_ascii_case("https");
    let local_http = scheme.eq_ignore_ascii_case("http") && is_loopback_host(authority);
    if !secure && !local_http {
        bail!(
            "[publish].endpoint must use https://; http:// is allowed only for localhost development"
        );
    }
    Ok(endpoint.to_string())
}

fn is_loopback_host(authority: &str) -> bool {
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(host) = host.strip_prefix('[') {
        host.split_once(']').map_or(host, |(host, _)| host)
    } else {
        host.split(':').next().unwrap_or_default()
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub(super) fn random_token(length: usize) -> Result<String> {
    let mut bytes = vec![0_u8; length];
    getrandom::fill(&mut bytes).context("OS random source unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(super) fn compress_snapshot(draft: &PublicationDraft) -> Result<Vec<u8>> {
    ensure_snapshot_plaintext_limit(draft.canonical_json.len())?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(draft.canonical_json.as_bytes())
        .context("failed to gzip publication snapshot")?;
    encoder
        .finish()
        .context("failed to finish publication snapshot compression")
}

pub(super) fn ensure_snapshot_plaintext_limit(len: usize) -> Result<()> {
    if len > SNAPSHOT_PLAINTEXT_LIMIT {
        bail!(
            "publication snapshot is {len} bytes uncompressed; maximum is 16 MiB; narrow the WorkSet"
        );
    }
    Ok(())
}

pub(super) fn encrypt_snapshot(compressed: Vec<u8>, id: &str, viewer_key: &str) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0_u8; 12];
    getrandom::fill(&mut nonce_bytes).context("OS random source unavailable")?;
    encrypt_snapshot_with_nonce(compressed, id, viewer_key, nonce_bytes)
}

pub(super) fn encrypt_snapshot_with_nonce(
    mut compressed: Vec<u8>,
    id: &str,
    viewer_key: &str,
    nonce_bytes: [u8; 12],
) -> Result<Vec<u8>> {
    let key_bytes = URL_SAFE_NO_PAD
        .decode(viewer_key)
        .context("invalid generated viewer key")?;
    let key = Key::<Aes256Gcm>::try_from(key_bytes.as_slice())
        .context("invalid generated viewer key length")?;
    let cipher = Aes256Gcm::new(&key);
    let nonce = Nonce::try_from(nonce_bytes.as_slice()).context("invalid publication nonce")?;
    let aad = format!("sivtr-publication-v1:{id}");
    let tag = cipher
        .encrypt_inout_detached(&nonce, aad.as_bytes(), compressed.as_mut_slice().into())
        .map_err(|_| anyhow::anyhow!("AES-GCM encryption failed"))?;
    let mut envelope = Vec::with_capacity(8 + 2 + 12 + compressed.len() + tag.len());
    envelope.extend_from_slice(ENVELOPE_MAGIC);
    envelope.extend_from_slice(&[1, 1]); // envelope v1, gzip compression
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&compressed);
    envelope.extend_from_slice(tag.as_slice());
    if envelope.len() > ENVELOPE_LIMIT {
        bail!("encrypted publication envelope exceeds 5 MiB");
    }
    Ok(envelope)
}

pub(super) fn publication_url(endpoint: &str, id: &str, viewer_key: &str) -> String {
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

pub(super) fn upload(
    endpoint: &str,
    id: &str,
    management_token: &str,
    published_at: &str,
    envelope: &[u8],
) -> Result<()> {
    let url = format!("{endpoint}/api/v1/publications/{id}");
    let response = agent(Duration::from_secs(30))
        .put(&url)
        .header("Content-Type", "application/octet-stream")
        .header("X-Sivtr-Management-Token", management_token)
        .header("X-Sivtr-Published-At", published_at)
        .send(envelope)
        .with_context(|| format!("publication upload failed: {url}"))?;
    if !response.status().is_success() {
        bail!("publication upload returned HTTP {}", response.status());
    }
    Ok(())
}

pub(super) fn delete_remote(endpoint: &str, id: &str, management_token: &str) -> Result<()> {
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
