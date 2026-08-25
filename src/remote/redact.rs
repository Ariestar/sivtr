//! Shared secret redaction for the live remote-sharing transport.
//!
//! The implementation lives in `sivtr-core::privacy` so public publication
//! and device-to-device sharing cannot silently drift apart.

use sivtr_core::privacy;
use sivtr_core::record::{WorkPart, WorkPartData, WorkRecord};

pub fn redact_record(record: &WorkRecord) -> WorkRecord {
    let mut out = record.clone();
    out.title = privacy::redact_text(&out.title);
    out.parts = out.parts.into_iter().map(redact_part).collect();
    out
}

pub fn redact_part(mut part: WorkPart) -> WorkPart {
    match &mut part.data {
        WorkPartData::Prompt { content, ansi } | WorkPartData::Output { content, ansi } => {
            *content = privacy::redact_text(content);
            if let Some(value) = ansi {
                *value = privacy::redact_text(value);
            }
        }
        WorkPartData::Command { content }
        | WorkPartData::User { content }
        | WorkPartData::Assistant { content }
        | WorkPartData::Thinking { content }
        | WorkPartData::Error { content } => *content = privacy::redact_text(content),
        WorkPartData::ToolCall { tool, input, .. } => {
            if let Some(value) = tool {
                *value = privacy::redact_text(value);
            }
            privacy::redact_json(input);
        }
        WorkPartData::ToolResult { tool, output, .. } => {
            if let Some(value) = tool {
                *value = privacy::redact_text(value);
            }
            privacy::redact_json(output);
        }
        WorkPartData::Skill { skill, content } => {
            if let Some(value) = skill {
                *value = privacy::redact_text(value);
            }
            *content = privacy::redact_text(content);
        }
    }
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_token_formats() {
        assert_eq!(
            privacy::redact_text("token ghp_aBcDeF0123456789ghij"),
            "token [REDACTED]"
        );
        assert_eq!(
            privacy::redact_text("key=sk-abcd1234efgh5678ijkl"),
            "key=[REDACTED]"
        );
        assert_eq!(
            privacy::redact_text("Authorization: Bearer abcdef1234567890XYZ"),
            "Authorization: [REDACTED]"
        );
    }

    #[test]
    fn redacts_pem_private_key_blocks() {
        let input = "before -----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA1234567890\n-----END RSA PRIVATE KEY----- after";
        let out = privacy::redact_text(input);
        assert!(!out.contains("MIIEpAIBAA"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn does_not_redact_plain_text() {
        assert_eq!(
            privacy::redact_text("the build succeeded with 42 warnings"),
            "the build succeeded with 42 warnings"
        );
    }
}
