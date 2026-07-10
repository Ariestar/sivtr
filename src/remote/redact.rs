//! Basic secret redaction for served records.
//!
//! Terminal and agent output routinely contains credentials (API keys, tokens,
//! connection strings). Since `sivtr serve` exposes workspace memory over the
//! network, responses pass through here first. This is a coarse, best-effort
//! redactor — it is not a security boundary on its own. Strong authentication
//! and network scoping (localhost default, opt-in LAN bind) are the real
//! boundary; this layer reduces the blast radius if a secret slipped into
//! captured output.

use regex::Regex;
use sivtr_core::record::{WorkPart, WorkRecord};

const REDACTED: &str = "[REDACTED]";

/// Patterns that match common leaked credential formats. Intentionally narrow
/// to high-signal prefixes to avoid redacting ordinary text; matched spans are
/// replaced wholesale.
fn patterns() -> Vec<(&'static str, Regex)> {
    // Compiled once per call; the serve path is not hot, and keeping this lazy
    // avoids a module-level unwrap.
    vec![
        // GitHub personal access tokens / fine-grained
        ("github_pat", Regex::new(r"gh[pousr]_[A-Za-z0-9]{16,}").unwrap()),
        // OpenAI / Anthropic-style API keys
        ("openai_key", Regex::new(r"sk-[A-Za-z0-9]{16,}").unwrap()),
        // sivtr serve connection tokens (s- namespace) — redact our own tokens so a
        // generated token that leaks into captured output is masked too.
        ("sivtr_token", Regex::new(r"s-[A-Za-z0-9]{16,}").unwrap()),
        // Slack tokens
        ("slack_token", Regex::new(r"xox[abprs]-[A-Za-z0-9-]{10,}").unwrap()),
        // AWS access key ids
        ("aws_id", Regex::new(r"AKIA[0-9A-Z]{16}").unwrap()),
        // AWS secret access key assignments
        ("aws_secret", Regex::new(r#"(?i)aws_secret_access_key['"\s:=]+[A-Za-z0-9/+=]{40}"#).unwrap()),
        // Generic secret assignments with a value
        (
            "assigned_secret",
            Regex::new(r#"(?i)(api[_-]?key|token|password|secret|bearer)\s*[:=]\s*['"]?[A-Za-z0-9_\-./+=]{12,}['"]?"#).unwrap(),
        ),
        // Bearer tokens in Authorization headers
        ("bearer", Regex::new(r"(?i)bearer\s+[A-Za-z0-9_\-\.=]{16,}").unwrap()),
        // Private keys (PEM blocks) — whole block collapsed
        (
            "pem_key",
            Regex::new(r"-----BEGIN [A-Z ]+PRIVATE KEY-----[\s\S]*?-----END [A-Z ]+PRIVATE KEY-----").unwrap(),
        ),
    ]
}

/// Redact obvious secrets in a free-text field.
fn redact_text(value: &str, patterns: &[(&'static str, Regex)]) -> String {
    let mut current = value.to_string();
    for (_, re) in patterns {
        current = re.replace_all(&current, REDACTED).into_owned();
    }
    current
}

/// Return a clone of `record` with secret-bearing text fields redacted:
/// title, each part's text/ansi/label. Structural fields (refs, times, status)
/// are untouched.
pub fn redact_record(record: &WorkRecord) -> WorkRecord {
    let patterns = patterns();
    let mut out = record.clone();
    out.title = redact_text(&out.title, &patterns);
    out.parts = out
        .parts
        .into_iter()
        .map(|part| redact_part(part, &patterns))
        .collect();
    out
}

pub fn redact_part(mut part: WorkPart, patterns: &[(&'static str, Regex)]) -> WorkPart {
    part.text = redact_text(&part.text, patterns);
    if let Some(ansi) = part.ansi.take() {
        part.ansi = Some(redact_text(&ansi, patterns));
    }
    if let Some(label) = part.label.take() {
        // Labels are short and rarely carry secrets, but redact for consistency.
        part.label = Some(redact_text(&label, patterns));
    }
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_token_formats() {
        let patterns = patterns();
        assert_eq!(
            redact_text("token ghp_aBcDeF0123456789ghij", &patterns),
            "token [REDACTED]"
        );
        assert_eq!(
            redact_text("key=sk-abcd1234efgh5678ijkl", &patterns),
            "key=[REDACTED]"
        );
        assert_eq!(
            redact_text("token s-deadbeefcafef00d1234567890abcdef", &patterns),
            "token [REDACTED]"
        );
        assert_eq!(
            redact_text("Authorization: Bearer abcdef1234567890XYZ", &patterns),
            "Authorization: [REDACTED]"
        );
    }

    #[test]
    fn redacts_pem_private_key_blocks() {
        let patterns = patterns();
        let input = "before -----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEA1234567890
-----END RSA PRIVATE KEY----- after";
        let out = redact_text(input, &patterns);
        assert!(!out.contains("MIIEpAIBAA"));
        assert!(out.contains("[REDACTED]"));
        assert!(out.contains("before"));
        assert!(out.contains("after"));
    }

    #[test]
    fn does_not_redact_plain_text() {
        let patterns = patterns();
        let out = redact_text("the build succeeded with 42 warnings", &patterns);
        assert_eq!(out, "the build succeeded with 42 warnings");
    }
}
