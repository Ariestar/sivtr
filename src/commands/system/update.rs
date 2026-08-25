//! `sivtr update` — self-update from GitHub Releases.
//!
//! Also exposes [`latest_version`] for `sivtr doctor` to check the published
//! release without updating.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

use ureq::ResponseExt;

use crate::output;

const REPO: &str = "Ariestar/sivtr";
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Latest published release tag, e.g. `v0.6.0`.
///
/// Follows the `releases/latest` redirect to `/releases/tag/<tag>`, avoiding
/// the rate-limited GitHub API entirely.  No GitHub CLI required.
pub fn latest_version() -> Result<String> {
    let response = agent(CHECK_TIMEOUT)
        .get("https://github.com/Ariestar/sivtr/releases/latest")
        .header("User-Agent", "sivtr-update")
        .call()
        .context("cannot reach GitHub")?;

    Ok(release_tag_from_path(response.get_uri().path())?.to_string())
}

fn release_tag_from_path(path: &str) -> Result<&str> {
    let tag = path
        .rsplit_once("/releases/tag/")
        .with_context(|| format!("release redirect path is missing /releases/tag/: {path}"))?
        .1;
    if tag.is_empty() || tag.contains('/') {
        bail!("release redirect path contains an invalid tag: {path}");
    }
    Ok(tag)
}

pub fn execute() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let latest = latest_version()?;
    let latest = latest.trim_start_matches('v');

    if let (Some(cur), Some(lat)) = (parse_version(current), parse_version(latest)) {
        if lat <= cur {
            output::success(format!("sivtr {current} is up to date"));
            return Ok(());
        }
    }

    output::info(format!("updating sivtr {current} -> {latest}"));

    let (asset, ext) = resolve_target()?;
    let archive_name = format!("sivtr-v{latest}-{asset}.{ext}");
    let bin_name = if cfg!(windows) { "sivtr.exe" } else { "sivtr" };

    let agent = agent(DOWNLOAD_TIMEOUT);
    let checksums = download(
        &agent,
        &format!("https://github.com/{REPO}/releases/download/v{latest}/SHA256SUMS"),
    )?;
    let expected = checksum_for(&checksums, &archive_name)
        .with_context(|| format!("no SHA256 entry for {archive_name}"))?;

    let url = format!("https://github.com/{REPO}/releases/download/v{latest}/{archive_name}");
    let archive = download(&agent, &url)?;
    verify_sha256(&archive, &expected)?;

    let current_bin = std::env::current_exe().context("cannot locate current binary")?;
    let temp = tempfile::tempdir_in(
        current_bin
            .parent()
            .context("current binary has no parent directory")?,
    )
    .context("cannot create temp directory next to the binary")?;
    let new_bin = extract_binary(&archive, ext, bin_name, temp.path())?;
    replace_binary(&new_bin, &current_bin)?;

    output::success(format!("updated to sivtr {latest}"));
    Ok(())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "sivtr-update")
        .call()
        .with_context(|| format!("download failed: {url}"))?;
    let buf = response
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("failed to read {url}"))?;
    Ok(buf)
}

fn resolve_target() -> Result<(&'static str, &'static str)> {
    let (asset, ext) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => ("windows-x64", "zip"),
        ("linux", "x86_64") => ("linux-x64-musl", "tar.gz"),
        ("macos", "aarch64") => ("macos", "tar.gz"),
        (os, arch) => bail!(
            "no prebuilt binary for {os}-{arch}; install from source with `cargo install sivtr`"
        ),
    };
    Ok((asset, ext))
}

/// Extract the binary from a release archive into `dir`, returning its path.
fn extract_binary(
    archive: &[u8],
    ext: &str,
    bin_name: &str,
    dir: &Path,
) -> Result<std::path::PathBuf> {
    let bin_path = dir.join(bin_name);
    let mut found = false;

    if ext == "zip" {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
            .context("cannot open zip archive")?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).context("cannot read zip entry")?;
            if entry.name() == bin_name || entry.name().ends_with(&format!("/{bin_name}")) {
                let mut out =
                    std::fs::File::create(&bin_path).context("cannot create temp file")?;
                std::io::copy(&mut entry, &mut out).context("cannot extract binary from zip")?;
                found = true;
                break;
            }
        }
    } else {
        let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(archive));
        let mut tar = tar::Archive::new(gz);
        for entry in tar.entries().context("cannot read tar archive")? {
            let mut entry = entry.context("cannot read tar entry")?;
            if entry
                .path()
                .context("unsafe path in archive")?
                .ends_with(bin_name)
            {
                entry
                    .unpack(&bin_path)
                    .context("cannot extract binary from tar.gz")?;
                found = true;
                break;
            }
        }
    }

    if !found {
        bail!("binary {bin_name} not found in archive");
    }
    Ok(bin_path)
}

fn replace_binary(new_bin: &Path, current: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        // A running exe cannot be overwritten in place; move it aside first.
        let old = current.with_extension("exe.old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(current, &old)
            .with_context(|| format!("cannot move {} aside", current.display()))?;
        std::fs::rename(new_bin, current)
            .with_context(|| format!("cannot install new binary at {}", current.display()))?;
        let _ = std::fs::remove_file(&old);
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(new_bin, current)
            .with_context(|| format!("cannot install new binary at {}", current.display()))?;
    }
    Ok(())
}

fn checksum_for(checksums: &[u8], archive_name: &str) -> Option<String> {
    String::from_utf8_lossy(checksums).lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        (name == archive_name).then(|| hash.to_string())
    })
}

fn verify_sha256(data: &[u8], expected_hex: &str) -> Result<()> {
    let actual = hex(&Sha256::digest(data));
    if actual != expected_hex.to_ascii_lowercase() {
        bail!("SHA256 mismatch: expected {expected_hex}, got {actual}");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_handles_v_prefix() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("0.4.0"), Some((0, 4, 0)));
    }

    #[test]
    fn parse_version_rejects_non_numeric() {
        assert_eq!(parse_version("0.4"), None);
        assert_eq!(parse_version("0.4.0-beta"), None);
    }

    #[test]
    fn version_ordering_is_numeric() {
        assert!(parse_version("0.10.0") > parse_version("0.9.0"));
    }

    #[test]
    fn checksum_for_finds_entry() {
        let sums = b"abc  sivtr-v0.4.0-windows-x64.zip\ndef  sivtr-v0.4.0-linux-x64-musl.tar.gz\n";
        assert_eq!(
            checksum_for(sums, "sivtr-v0.4.0-windows-x64.zip"),
            Some("abc".to_string())
        );
        assert_eq!(checksum_for(sums, "missing.zip"), None);
    }

    #[test]
    fn tag_from_redirect_path() {
        assert_eq!(
            release_tag_from_path("/Ariestar/sivtr/releases/tag/v0.6.0").expect("valid tag"),
            "v0.6.0"
        );
        assert!(release_tag_from_path("/Ariestar/sivtr/releases/tag/").is_err());
        assert!(release_tag_from_path("/Ariestar/sivtr").is_err());
    }
}
