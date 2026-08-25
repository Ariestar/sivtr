//! Shared, best-effort privacy helpers.
//!
//! This module deliberately only removes high-signal credential formats.  It
//! is a reduction in accidental disclosure, not a security boundary: callers
//! must still ask the user to review the resulting snapshot before publishing.

use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const REDACTED: &str = "[REDACTED]";

static PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    vec![
        ("github_pat", Regex::new(r"gh[pousr]_[A-Za-z0-9]{16,}").unwrap()),
        ("openai_key", Regex::new(r"sk-[A-Za-z0-9]{16,}").unwrap()),
        ("sivtr_token", Regex::new(r"s-[A-Za-z0-9]{16,}").unwrap()),
        ("slack_token", Regex::new(r"xox[abprs]-[A-Za-z0-9-]{10,}").unwrap()),
        ("aws_id", Regex::new(r"AKIA[0-9A-Z]{16}").unwrap()),
        (
            "aws_secret",
            Regex::new(r#"(?i)aws_secret_access_key['"\s:=]+[A-Za-z0-9/+=]{40}"#)
                .unwrap(),
        ),
        (
            "assigned_secret",
            Regex::new(r#"(?i)(api[_-]?key|token|password|secret|bearer)\s*[:=]\s*['"]?[A-Za-z0-9_\-./+=]{12,}['"]?"#)
                .unwrap(),
        ),
        (
            "bearer",
            Regex::new(r"(?i)bearer\s+[A-Za-z0-9_\-.=]{16,}").unwrap(),
        ),
        (
            "pem_key",
            Regex::new(r"-----BEGIN [A-Z ]+PRIVATE KEY-----[\s\S]*?-----END [A-Z ]+PRIVATE KEY-----")
                .unwrap(),
        ),
    ]
});

static WARNING_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    vec![
        (
            "absolute_path",
            Regex::new(r#"(?i)(?:[A-Z]:[\\/]|/(?:Users|home|tmp|var|etc)/|\\\\)[^\s`]+"#)
                .unwrap(),
        ),
        ("email", Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").unwrap()),
        ("internal_url", Regex::new(r"(?i)https?://(?:localhost|127\.0\.0\.1|10\.(?:[0-9]{1,3}\.){2}[0-9]{1,3}|192\.168\.(?:[0-9]{1,3}\.)[0-9]{1,3}|[A-Za-z0-9-]+\.local)(?::\d+)?(?:/[^\s]*)?").unwrap()),
    ]
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextPrivacyReport {
    pub redactions: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyReport {
    pub redactions: usize,
    pub warnings: Vec<PrivacyWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyWarning {
    pub kind: String,
    pub item_index: usize,
}

/// Redact high-signal credentials and report non-mutating disclosure risks.
pub fn redact_text_with_report(value: &str) -> (String, TextPrivacyReport) {
    let mut current = value.to_string();
    let mut report = TextPrivacyReport::default();
    for (name, regex) in PATTERNS.iter() {
        let count = regex.find_iter(&current).count();
        if count > 0 {
            report.redactions += count;
            current = regex.replace_all(&current, REDACTED).into_owned();
            report.warnings.push((*name).to_string());
        }
    }
    for (name, regex) in WARNING_PATTERNS.iter() {
        if regex.is_match(&current) {
            report.warnings.push((*name).to_string());
        }
    }
    report.warnings.sort();
    report.warnings.dedup();
    (current, report)
}

/// Shared redaction entry point used by the existing remote sharing path.
pub fn redact_text(value: &str) -> String {
    redact_text_with_report(value).0
}

/// Redact every textual value in a JSON tool payload.  Kept public so future
/// transports can reuse the exact same credential patterns.
pub fn redact_json(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact_text(text),
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        Value::Object(object) => object.values_mut().for_each(redact_json),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Count warning-only risks in a public item and attach the item position.
pub fn warnings_for_item(text: &str, item_index: usize) -> Vec<PrivacyWarning> {
    let (_, report) = redact_text_with_report(text);
    report
        .warnings
        .into_iter()
        .filter(|kind| {
            kind != "github_pat"
                && kind != "openai_key"
                && kind != "sivtr_token"
                && kind != "slack_token"
                && kind != "aws_id"
                && kind != "aws_secret"
                && kind != "assigned_secret"
                && kind != "bearer"
                && kind != "pem_key"
        })
        .map(|kind| PrivacyWarning { kind, item_index })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_existing_remote_token_shapes() {
        let (text, report) =
            redact_text_with_report("ghp_aBcDeF0123456789ghij sk-abcd1234efgh5678ijkl");
        assert_eq!(text, "[REDACTED] [REDACTED]");
        assert_eq!(report.redactions, 2);
    }

    #[test]
    fn warns_without_changing_paths_and_emails() {
        let (text, report) = redact_text_with_report(r"C:\Users\alice\repo alice@example.com");
        assert_eq!(text, r"C:\Users\alice\repo alice@example.com");
        assert!(report.warnings.iter().any(|item| item == "absolute_path"));
        assert!(report.warnings.iter().any(|item| item == "email"));
    }
}
