//! Provider-neutral, privacy-minimized public conversation snapshots.

use anyhow::{bail, ensure, Result};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::privacy;
use crate::record::{WorkPartKind, WorkRecord, WorkRecordKind, WorkRef};

pub const PUBLICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PublicationExpiry {
    TwoHours,
    OneDay,
    ThreeDays,
    #[default]
    SevenDays,
    ThirtyDays,
}

impl PublicationExpiry {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "2h" => Ok(Self::TwoHours),
            "1d" => Ok(Self::OneDay),
            "3d" => Ok(Self::ThreeDays),
            "7d" => Ok(Self::SevenDays),
            "30d" => Ok(Self::ThirtyDays),
            _ => {
                bail!("invalid publication expiry `{value}`; expected 2h, 1d, 3d, 7d, or 30d")
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TwoHours => "2h",
            Self::OneDay => "1d",
            Self::ThreeDays => "3d",
            Self::SevenDays => "7d",
            Self::ThirtyDays => "30d",
        }
    }

    fn duration(self) -> Duration {
        match self {
            Self::TwoHours => Duration::hours(2),
            Self::OneDay => Duration::days(1),
            Self::ThreeDays => Duration::days(3),
            Self::SevenDays => Duration::days(7),
            Self::ThirtyDays => Duration::days(30),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PublicationPolicy {
    pub title: Option<String>,
    pub expires: PublicationExpiry,
    /// Injectable for deterministic tests; production callers leave this None.
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationV1 {
    pub schema_version: u32,
    pub title: String,
    pub provider: String,
    pub published_at: String,
    pub expires_at: String,
    pub items: Vec<PublicConversationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationItem {
    pub role: PublicRole,
    pub text: String,
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PublicRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationRisk {
    pub kind: String,
    pub count: usize,
    pub item_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct PublicationDraft {
    pub snapshot: PublicConversationV1,
    pub canonical_json: String,
    pub content_sha256: String,
    pub redaction_count: usize,
    pub risks: Vec<PublicationRisk>,
    pub source_provider: String,
    pub source_refs: Vec<String>,
}

impl PublicationDraft {
    pub fn item_count(&self) -> usize {
        self.snapshot.items.len()
    }

    pub fn turn_count(&self) -> usize {
        self.source_refs.len()
    }
}

/// Validate and project a WorkSet's materialized records into the only public
/// shape supported by v1.  The core never receives a CLI WorkSet type.
pub fn create_publication_draft(
    records: &[WorkRecord],
    anchors: &[WorkRef],
    policy: &PublicationPolicy,
) -> Result<PublicationDraft> {
    ensure!(!records.is_empty(), "cannot publish an empty WorkSet");
    let expected = if anchors.is_empty() {
        records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect()
    } else {
        ensure!(
            anchors.iter().all(|anchor| anchor.part().is_none()),
            "publish v1 requires record-level anchors, not part anchors"
        );
        anchors.iter().map(WorkRef::whole).collect::<Vec<_>>()
    };
    ensure!(
        expected.len() == records.len(),
        "publication anchors and records must have the same length"
    );

    // Search defaults to newest-first; publish snapshots are chronological.
    let mut order: Vec<usize> = (0..records.len()).collect();
    order.sort_by_key(|&i| records[i].work_ref.index());

    let first = &records[order[0]];
    ensure!(
        first.kind == WorkRecordKind::ChatTurn,
        "publish v1 only supports agent conversations, not terminal records"
    );
    ensure!(
        first.work_ref.is_local(),
        "publish v1 only supports local WorkSets"
    );
    ensure!(
        first.work_ref.part().is_none(),
        "publish v1 requires record-level anchors, not part anchors"
    );
    let provider = first
        .work_ref
        .provider()
        .ok_or_else(|| anyhow::anyhow!("publish v1 requires an agent provider"))?;
    let session = first.work_ref.session().to_string();
    let mut source_refs = Vec::with_capacity(records.len());
    let mut items = Vec::new();
    let mut redaction_count = 0;
    let mut risk_map: std::collections::BTreeMap<String, PublicationRisk> =
        std::collections::BTreeMap::new();
    let mut previous_index = None;

    for &idx in &order {
        let record = &records[idx];
        ensure!(
            record.work_ref.whole() == expected[idx],
            "publication anchors must match records in order"
        );
        ensure!(
            record.kind == WorkRecordKind::ChatTurn,
            "publish v1 only supports agent conversations"
        );
        ensure!(
            record.source.channel == crate::record::WorkChannel::Chat,
            "publication contains a non-chat record"
        );
        ensure!(
            record.source.provider.as_deref() == Some(provider.command_name()),
            "publication provider metadata does not match its WorkRef"
        );
        ensure!(
            record.work_ref.is_local(),
            "publication contains a remote or group record"
        );
        ensure!(
            record.work_ref.part().is_none(),
            "publication anchors must target whole records"
        );
        ensure!(
            record.work_ref.provider() == Some(provider),
            "publication cannot mix agent providers"
        );
        ensure!(
            record.work_ref.session() == session,
            "publication cannot mix agent sessions"
        );
        if let Some(previous) = previous_index {
            ensure!(
                record.work_ref.index() == previous + 1,
                "publication record indices must be strictly continuous"
            );
        }
        previous_index = Some(record.work_ref.index());
        source_refs.push(record.work_ref.to_string());

        for part in &record.parts {
            let role = match part.kind() {
                WorkPartKind::User => PublicRole::User,
                WorkPartKind::Assistant => PublicRole::Assistant,
                _ => continue,
            };
            let raw = part.text().into_owned();
            let (text, report) = privacy::redact_text_with_report(&raw)?;
            redaction_count += report.redactions;
            let item_index = (!text.trim().is_empty()).then_some(items.len() + 1);
            for kind in report.warnings {
                let entry = risk_map
                    .entry(kind.clone())
                    .or_insert_with(|| PublicationRisk {
                        kind,
                        count: 0,
                        item_indices: Vec::new(),
                    });
                entry.count += 1;
                if let Some(item_index) = item_index {
                    entry.item_indices.push(item_index);
                }
            }
            if !text.trim().is_empty() {
                items.push(PublicConversationItem {
                    role,
                    text,
                    occurred_at: part
                        .occurred_at
                        .clone()
                        .or_else(|| record.time.primary_at().map(str::to_string)),
                });
            }
        }
    }
    ensure!(
        items.iter().any(|item| item.role == PublicRole::Assistant),
        "publication must contain at least one assistant reply"
    );

    let now = policy.published_at.unwrap_or_else(Utc::now);
    let expires_at = now + policy.expires.duration();
    let title_raw = policy
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| first.title.clone());
    let (title, title_report) = privacy::redact_text_with_report(&title_raw)?;
    redaction_count += title_report.redactions;
    for kind in title_report.warnings {
        let entry = risk_map
            .entry(kind.clone())
            .or_insert_with(|| PublicationRisk {
                kind,
                count: 0,
                item_indices: Vec::new(),
            });
        entry.count += 1;
    }
    let snapshot = PublicConversationV1 {
        schema_version: PUBLICATION_SCHEMA_VERSION,
        title: if title.trim().is_empty() {
            "Sivtr conversation".to_string()
        } else {
            title
        },
        provider: provider.command_name().to_string(),
        published_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        expires_at: expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        items,
    };
    let canonical_json = serde_json::to_string(&snapshot)?;
    let content_sha256 = hex_sha256(canonical_json.as_bytes());
    let risks = risk_map
        .into_values()
        .map(|mut risk| {
            risk.item_indices.sort_unstable();
            risk.item_indices.dedup();
            risk
        })
        .collect();
    Ok(PublicationDraft {
        snapshot,
        canonical_json,
        content_sha256,
        redaction_count,
        risks,
        source_provider: provider.command_name().to_string(),
        source_refs,
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkRef, WorkSessionRef, WorkSource,
        WorkTime,
    };

    fn record(index: usize, assistant: &str) -> WorkRecord {
        WorkRecord {
            schema_version: 3,
            work_ref: WorkRef::agent(crate::ai::AgentProvider::Codex, "session", index),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: WorkSessionRef {
                id: "session".into(),
                canonical_id: None,
                path: None,
            },
            cwd: Some("C:\\secret".into()),
            time: WorkTime::default(),
            status: None,
            title: "Demo".into(),
            parts: vec![
                WorkPart {
                    seq: 1,
                    occurred_at: None,
                    data: WorkPartData::User {
                        content: "hello".into(),
                    },
                },
                WorkPart {
                    seq: 2,
                    occurred_at: None,
                    data: WorkPartData::Assistant {
                        content: assistant.into(),
                    },
                },
            ],
        }
    }

    #[test]
    fn projects_only_dialogue_and_redacts_secrets() {
        let records = vec![record(3, "token=sk-abcd1234efgh5678ijkl")];
        let draft = create_publication_draft(&records, &[], &PublicationPolicy::default()).unwrap();
        assert_eq!(draft.item_count(), 2);
        assert_eq!(draft.snapshot.items[1].text, "token=[REDACTED]");
        assert_eq!(draft.redaction_count, 1);
        let json = serde_json::to_string(&draft.snapshot).unwrap();
        assert!(!json.contains("work_ref"));
        assert!(!json.contains("cwd"));
        assert!(!json.contains("session"));
    }

    #[test]
    fn rejects_gaps_and_mixed_sessions() {
        let records = vec![record(1, "a"), record(3, "b")];
        assert!(create_publication_draft(&records, &[], &PublicationPolicy::default()).is_err());
        let mut mixed = record(2, "b");
        mixed.work_ref = WorkRef::agent(crate::ai::AgentProvider::Codex, "other", 2);
        assert!(create_publication_draft(
            &[record(1, "a"), mixed],
            &[],
            &PublicationPolicy::default()
        )
        .is_err());
        assert!(create_publication_draft(
            &[record(1, "a")],
            &[WorkRef::agent(crate::ai::AgentProvider::Codex, "session", 1).with_part(1)],
            &PublicationPolicy::default()
        )
        .is_err());
    }

    #[test]
    fn newest_first_records_are_sorted_before_continuity_check() {
        let records = vec![record(3, "c"), record(2, "b"), record(1, "a")];
        let draft = create_publication_draft(&records, &[], &PublicationPolicy::default()).unwrap();
        assert_eq!(draft.turn_count(), 3);
        assert_eq!(
            draft.source_refs,
            vec![
                "codex/session/1".to_string(),
                "codex/session/2".to_string(),
                "codex/session/3".to_string(),
            ]
        );
        let assistant: Vec<_> = draft
            .snapshot
            .items
            .iter()
            .filter(|item| item.role == PublicRole::Assistant)
            .map(|item| item.text.as_str())
            .collect();
        assert_eq!(assistant, ["a", "b", "c"]);
    }

    #[test]
    fn publication_expiry_parses_supported_lifetimes() {
        assert_eq!(
            PublicationExpiry::parse("2h").unwrap(),
            PublicationExpiry::TwoHours
        );
        assert_eq!(
            PublicationExpiry::parse("3d").unwrap(),
            PublicationExpiry::ThreeDays
        );
        assert_eq!(PublicationExpiry::TwoHours.as_str(), "2h");
        assert_eq!(PublicationExpiry::ThreeDays.as_str(), "3d");
        assert_eq!(PublicationExpiry::TwoHours.duration(), Duration::hours(2));
        assert_eq!(PublicationExpiry::ThreeDays.duration(), Duration::days(3));
        assert!(PublicationExpiry::parse("4h").is_err());
        assert!(PublicationExpiry::parse("90d").is_err());
    }
}
