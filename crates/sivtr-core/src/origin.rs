//! Unified source origins.
//!
//! Every memory source — a local workspace, a remote device mount, a cloud
//! account — is described by one [`Origin`] with the same four fields, so
//! upper layers (listing, rendering) never branch on which kind of source
//! they are looking at. Kind-specific details (root paths, peer/share ids,
//! cloud account) never enter [`Origin`]: the display [`Origin::detail`] is
//! composed by the source at construction time, and whether a remote or
//! cloud source happens to be ingested into local files is a
//! resolution-layer concern, not an origin concern.
//!
//! [`OriginRegistry`] is the single lookup surface: enumerate every
//! addressable origin, or resolve one by its logical name. Each entry pairs
//! the display [`Origin`] with its [`Reach`] — the kind-specific payload the
//! resolution layer needs to actually load data. Display layers only ever
//! see [`Origin`]; the resolution layer dispatches on [`Reach`].

use anyhow::{bail, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Major source category.
///
/// `#[non_exhaustive]`: adding a category (WSL, container, archive, …) only
/// requires a new variant plus a `label()` arm here; code outside this crate
/// is forced to handle the wildcard, so nothing downstream breaks.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OriginKind {
    /// Local files on this machine (workspaces).
    Local,
    /// Another device, forwarded through the daemon.
    Remote,
    /// A cloud account (synced and/or fetched).
    Cloud,
}

impl OriginKind {
    /// Stable lowercase label for display and serialization.
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Cloud => "cloud",
        }
    }
}

/// A single addressable memory source.
///
/// All fields exist for every kind; `detail` is the display projection the
/// source composed when it was constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    /// Logical name (scope name / alias), used by [`OriginRegistry::resolve`].
    pub name: String,
    /// Major source category.
    pub kind: OriginKind,
    /// Whether this origin is the current context (e.g. the current workspace).
    pub current: bool,
    /// Display projection composed by the source (e.g. `root (key)`).
    pub detail: String,
}

/// How to reach an origin's data. Resolution-layer only: carries the
/// kind-specific payload display layers never see. Exhaustive, so adding a
/// kind forces every resolution dispatch to gain an arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// A workspace on this machine: its root directory.
    Local { root: String },
    /// A mount on another device: which workspace's mount list and which alias.
    Remote {
        workspace_key: String,
        alias: String,
    },
    /// A cloud account (reserved — none yet).
    Cloud,
}

/// One registry entry: the display [`Origin`] plus its [`Reach`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub origin: Origin,
    pub reach: Reach,
}

/// Every origin addressable from the current context.
///
/// A pure resolution view: sources construct their own [`Entry`]s and hand
/// them in; this type owns no I/O and never interprets kind-specific fields.
#[derive(Debug, Clone, Default)]
pub struct OriginRegistry {
    entries: Vec<Entry>,
}

impl OriginRegistry {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self { entries }
    }

    /// Display origins in construction order.
    pub fn all(&self) -> impl Iterator<Item = &Origin> + '_ {
        self.entries.iter().map(|entry| &entry.origin)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolve an origin by logical name, case-insensitively, returning its
    /// entry (display [`Origin`] + [`Reach`]). Errors when more than one
    /// origin shares the name.
    pub fn resolve(&self, name: &str) -> Result<Option<&Entry>> {
        let mut matched = self
            .entries
            .iter()
            .filter(|entry| entry.origin.name.eq_ignore_ascii_case(name));
        let Some(first) = matched.next() else {
            return Ok(None);
        };
        if let Some(second) = matched.next() {
            let mut details = vec![first.origin.detail.as_str(), second.origin.detail.as_str()];
            details.extend(matched.map(|entry| entry.origin.detail.as_str()));
            bail!("ambiguous origin `{name}`; matches: {}", details.join(", "));
        }
        Ok(Some(first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OriginRegistry {
        OriginRegistry::new(vec![
            Entry {
                origin: Origin {
                    name: "sivtr".to_string(),
                    kind: OriginKind::Local,
                    current: true,
                    detail: "D:\\Coding\\sivtr (key1)".to_string(),
                },
                reach: Reach::Local {
                    root: "D:\\Coding\\sivtr".to_string(),
                },
            },
            Entry {
                origin: Origin {
                    name: "desk".to_string(),
                    kind: OriginKind::Remote,
                    current: false,
                    detail: "alice/sivtr".to_string(),
                },
                reach: Reach::Remote {
                    workspace_key: "key1".to_string(),
                    alias: "desk".to_string(),
                },
            },
        ])
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(OriginKind::Local.label(), "local");
        assert_eq!(OriginKind::Remote.label(), "remote");
        assert_eq!(OriginKind::Cloud.label(), "cloud");
    }

    #[test]
    fn resolve_matches_name_case_insensitively() {
        let registry = sample();
        assert_eq!(
            registry
                .resolve("desk")
                .expect("resolve")
                .map(|entry| entry.origin.name.as_str()),
            Some("desk")
        );
        assert_eq!(
            registry
                .resolve("DESK")
                .expect("resolve")
                .map(|entry| entry.origin.name.as_str()),
            Some("desk")
        );
        assert_eq!(
            registry
                .resolve("Sivtr")
                .expect("resolve")
                .map(|entry| entry.origin.name.as_str()),
            Some("sivtr")
        );
        assert_eq!(registry.resolve("missing").expect("resolve"), None);
    }

    #[test]
    fn resolve_errors_on_ambiguous_names() {
        let registry = OriginRegistry::new(vec![
            Entry {
                origin: Origin {
                    name: "return".to_string(),
                    kind: OriginKind::Local,
                    current: false,
                    detail: "D:\\a (k1)".to_string(),
                },
                reach: Reach::Local {
                    root: "D:\\a".to_string(),
                },
            },
            Entry {
                origin: Origin {
                    name: "return".to_string(),
                    kind: OriginKind::Local,
                    current: false,
                    detail: "D:\\b (k2)".to_string(),
                },
                reach: Reach::Local {
                    root: "D:\\b".to_string(),
                },
            },
        ]);
        let error = registry.resolve("return").expect_err("ambiguous");
        assert!(error.to_string().contains("ambiguous origin `return`"));
    }

    #[test]
    fn resolve_returns_reach_payload_per_kind() {
        let registry = sample();
        let entry = registry.resolve("desk").expect("resolve").expect("found");
        assert!(matches!(
            &entry.reach,
            Reach::Remote { alias, .. } if alias == "desk"
        ));
        let entry = registry.resolve("sivtr").expect("resolve").expect("found");
        assert!(matches!(
            &entry.reach,
            Reach::Local { root } if root == "D:\\Coding\\sivtr"
        ));
    }

    #[test]
    fn all_preserves_construction_order() {
        let registry = sample();
        let names: Vec<_> = registry.all().map(|origin| origin.name.as_str()).collect();
        assert_eq!(names, vec!["sivtr", "desk"]);
    }

    #[test]
    fn origin_fields_are_uniform_across_kinds() {
        // Upper layers read the same four fields no matter the kind.
        for origin in sample().all() {
            assert!(!origin.name.is_empty());
            assert!(!origin.detail.is_empty());
            assert!(matches!(origin.kind.label(), "local" | "remote" | "cloud"));
        }
    }

    #[test]
    fn kind_serializes_as_lowercase_label() {
        assert_eq!(
            serde_json::to_string(&OriginKind::Remote).expect("serialize kind"),
            "\"remote\""
        );
        let origin = sample().all().nth(1).expect("second origin").clone();
        let json = serde_json::to_string(&origin).expect("serialize origin");
        assert!(json.contains("\"kind\":\"remote\""));
        let round: Origin = serde_json::from_str(&json).expect("deserialize origin");
        assert_eq!(round, origin);
    }
}
