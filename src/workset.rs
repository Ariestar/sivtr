//! Canonical record and part selection shared by the TUI, CLI, and MCP.

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};
use std::collections::HashSet;

pub const WORKSET_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkSet {
    pub schema_version: u32,
    pub created_at: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub records: Vec<WorkRecord>,
    pub anchors: Vec<WorkRef>,
}

impl WorkSet {
    pub fn new(cwd: impl Into<String>, records: Vec<WorkRecord>) -> Self {
        let anchors = records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect();
        Self::with_anchors(cwd, records, anchors)
    }

    pub fn with_anchors(
        cwd: impl Into<String>,
        records: Vec<WorkRecord>,
        anchors: Vec<WorkRef>,
    ) -> Self {
        Self {
            schema_version: WORKSET_SCHEMA_VERSION,
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            cwd: cwd.into(),
            name: None,
            records,
            anchors,
        }
    }

    pub fn anchors(&self) -> Vec<WorkRef> {
        self.anchors.clone()
    }

    /// Whether this selection covers an address. A Whole anchor covers every
    /// Part of the same record.
    pub fn contains(&self, anchor: &WorkRef) -> bool {
        self.anchors().iter().any(|selected| {
            selected == anchor
                || (selected.at == WorkAt::Whole && selected.whole() == anchor.whole())
        })
    }

    /// Include a complete record. Whole is the canonical representation for a
    /// record-wide selection, so any narrower anchors for it are removed.
    pub fn include_whole(&mut self, record: WorkRecord) {
        let whole = record.work_ref.whole();
        self.records.retain(|item| item.work_ref.whole() != whole);
        self.records.push(record);
        self.anchors.retain(|anchor| anchor.whole() != whole);
        self.anchors.push(whole);
    }

    pub fn include_records(&mut self, records: impl IntoIterator<Item = WorkRecord>) {
        for record in records {
            let whole = record.work_ref.whole();
            if !self
                .anchors
                .iter()
                .any(|anchor| anchor.at == WorkAt::Whole && anchor.whole() == whole)
            {
                self.include_whole(record);
            }
        }
    }

    /// Include selected parts of a record. A Whole anchor already covers them
    /// and remains the only stored representation.
    pub fn include_parts(&mut self, record: WorkRecord, parts: impl IntoIterator<Item = usize>) {
        let parts: Vec<_> = parts.into_iter().collect();
        let whole = record.work_ref.whole();
        if !record.parts.is_empty() && record.parts.iter().all(|part| parts.contains(&part.seq)) {
            self.include_whole(record);
            return;
        }
        if self
            .anchors()
            .iter()
            .any(|anchor| anchor.at == WorkAt::Whole && anchor.whole() == whole)
        {
            return;
        }
        if !self
            .records
            .iter()
            .any(|item| item.work_ref.whole() == whole)
        {
            self.records.push(record.clone());
        }
        for seq in parts {
            let anchor = whole.with_part(seq);
            if !self.anchors.contains(&anchor) {
                self.anchors.push(anchor);
            }
        }
    }

    /// Toggle a complete record selection.
    pub fn toggle_whole(&mut self, record: WorkRecord) {
        if self
            .anchors()
            .iter()
            .any(|anchor| anchor.at == WorkAt::Whole && anchor.whole() == record.work_ref.whole())
        {
            self.exclude(&record.work_ref);
        } else {
            self.include_whole(record);
        }
    }

    /// Toggle a set of parts. A fully selected set is removed; otherwise the
    /// missing parts are added and a Whole selection remains authoritative.
    pub fn toggle_parts(&mut self, record: WorkRecord, parts: impl IntoIterator<Item = usize>) {
        let parts: Vec<_> = parts.into_iter().collect();
        if parts.is_empty() {
            return;
        }
        if !record.parts.is_empty() && record.parts.iter().all(|part| parts.contains(&part.seq)) {
            self.toggle_whole(record);
            return;
        }
        if parts
            .iter()
            .all(|seq| self.contains(&record.work_ref.with_part(*seq)))
        {
            let whole = record.work_ref.whole();
            let remove: HashSet<_> = parts.into_iter().map(|seq| whole.with_part(seq)).collect();
            self.anchors.retain(|anchor| !remove.contains(anchor));
            if !self.anchors.iter().any(|anchor| anchor.whole() == whole) {
                self.records.retain(|item| item.work_ref.whole() != whole);
            }
        } else {
            self.include_parts(record, parts);
        }
    }

    /// Toggle every record in a scope using the same Whole rule.
    pub fn toggle_records(&mut self, records: Vec<WorkRecord>) {
        if records.is_empty() {
            return;
        }
        let all_selected = records.iter().all(|record| {
            self.anchors().iter().any(|anchor| {
                anchor.at == WorkAt::Whole && anchor.whole() == record.work_ref.whole()
            })
        });
        if all_selected {
            for record in records {
                self.exclude(&record.work_ref);
            }
        } else {
            for record in records {
                self.include_whole(record);
            }
        }
    }

    /// The part-only subset, preserving anchor order. Whole anchors do not
    /// participate in a block-copy operation.
    pub fn parts_only(&self) -> Option<Self> {
        let anchors: Vec<_> = self
            .anchors()
            .into_iter()
            .filter(|anchor| anchor.at != WorkAt::Whole)
            .collect();
        if anchors.is_empty() {
            return None;
        }
        let records = self
            .records
            .iter()
            .filter(|record| {
                anchors
                    .iter()
                    .any(|anchor| anchor.whole() == record.work_ref.whole())
            })
            .cloned()
            .collect();
        Some(Self::with_anchors(self.cwd.clone(), records, anchors))
    }

    /// Remove a record and every Part anchor belonging to it.
    pub fn exclude(&mut self, record_ref: &WorkRef) {
        let whole = record_ref.whole();
        self.records
            .retain(|record| record.work_ref.whole() != whole);
        self.anchors.retain(|anchor| anchor.whole() != whole);
    }
}
