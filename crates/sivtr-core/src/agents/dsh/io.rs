use super::PROVIDER_NAME;
use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::Path;

/// Decompressed byte cap for listing metadata reads (covers the header frame
/// and the first flush batch, where the title normally lands).
const META_READ_CAP: usize = 4 * 1024 * 1024;

/// Compressed byte cap for the same metadata read. The header is always the
/// first line of the first frame, so a torn first frame still yields id/cwd.
const META_COMPRESSED_READ_CAP: u64 = 8 * 1024 * 1024;

/// Maximum accepted zstd window. Node's built-in zstd (what dsh uses) stays
/// far below this; the generous cap avoids truncating sessions re-encoded
/// with a high-level zstd CLI. Decoding is local, trusted input.
const MAX_ZSTD_WINDOW: u64 = 256 * 1024 * 1024;

/// Read a whole session log as text, decompressing `.jsonl.zstd` artifacts.
pub(super) fn read_log(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path)
        .with_context(|| format!("Failed to read {PROVIDER_NAME} session: {}", path.display()))?;
    if is_zstd(path) {
        decode_zstd(&bytes, None).with_context(|| {
            format!(
                "Failed to decompress {PROVIDER_NAME} session: {}",
                path.display()
            )
        })
    } else {
        Ok(bytes)
    }
}

/// Read the head of a session log for listing metadata, decompressing only
/// the leading compressed bytes of a `.jsonl.zstd` artifact.
pub(super) fn read_log_head(path: &Path) -> Result<String> {
    let compressed = is_zstd(path);
    let mut file = fs::File::open(path)
        .with_context(|| format!("Failed to read {PROVIDER_NAME} session: {}", path.display()))?;
    let cap = if compressed {
        META_COMPRESSED_READ_CAP
    } else {
        META_READ_CAP as u64
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(cap)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Failed to read {PROVIDER_NAME} session: {}", path.display()))?;
    let bytes = if compressed {
        decode_zstd(&bytes, Some(META_READ_CAP)).with_context(|| {
            format!(
                "Failed to decompress {PROVIDER_NAME} session: {}",
                path.display()
            )
        })?
    } else {
        bytes
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn is_zstd(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("zstd")
}

/// Decompress a concatenation of independent Zstandard frames (dsh's layout:
/// one checksummed frame for the header line, then one frame per flush
/// batch). A torn or unreadable trailing frame is dropped, mirroring the
/// JSONL backend's crash-recovery truncation; `cap` bounds decompressed
/// output for head reads.
pub(super) fn decode_zstd(bytes: &[u8], cap: Option<usize>) -> Result<Vec<u8>> {
    use ruzstd::decoding::StreamingDecoder;

    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() && !cap.is_some_and(|cap| out.len() >= cap) {
        let mut cursor = std::io::Cursor::new(rest);
        let mut decoder =
            match StreamingDecoder::new_with_max_window_size(&mut cursor, MAX_ZSTD_WINDOW) {
                // Not a readable frame header: torn tail or trailing bytes. Keep
                // the complete frames decoded so far.
                Err(_) => break,
                Ok(decoder) => decoder,
            };
        let mut frame = Vec::new();
        let limit = cap.map(|cap| cap.saturating_sub(out.len()));
        let result = match limit {
            Some(limit) => decoder.by_ref().take(limit as u64).read_to_end(&mut frame),
            None => decoder.read_to_end(&mut frame),
        };
        let consumed = decoder.get_ref().position() as usize;
        match result {
            Ok(_) => {
                out.extend_from_slice(&frame);
                if consumed == 0 {
                    break;
                }
                rest = &rest[consumed..];
            }
            // Torn frame body: keep only the frames completed before it.
            Err(_) => break,
        }
    }
    Ok(out)
}
