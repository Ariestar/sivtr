//! Unified source origins.
//!
//! Every memory source — a local workspace or a remote device mount — is
//! described by one [`Origin`] with the same four fields, so upper layers
//! (listing, rendering) never branch on which kind of source they are
//! looking at. Kind-specific details (root paths, peer/share ids) never
//! enter [`Origin`]: the display [`Origin::detail`] is composed by the source
//! at construction time, and whether a remote source happens to be ingested
//! into local files is a resolution-layer concern, not an origin concern.
//!
//! [`OriginRegistry`] is the single lookup surface: enumerate every
//! addressable origin, or resolve one by its logical name. Each entry pairs
//! the display [`Origin`] with its [`Reach`] — the kind-specific payload the
//! resolution layer needs to actually load data. Display layers only ever
//! see [`Origin`]; the resolution layer dispatches on [`Reach`].

use anyhow::{bail, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A single addressable memory source.
///
/// All fields exist for every kind; `detail` is the display projection the
/// source composed when it was constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    /// Logical name (scope name / alias), used by [`OriginRegistry::resolve`].
    pub name: String,
    /// Stable lowercase label of the reach kind (`"local"` / `"remote"`).
    /// Private: derived from [`Reach`] by [`Entry::new`], so classification
    /// always agrees with the reach. Serialized for MCP consumers.
    kind: String,
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
    /// A mount on another device: which workspace's mount list.
    /// The mount alias is [`Origin::name`].
    Remote { workspace_key: String },
}

impl Reach {
    /// Stable lowercase label for display and serialization.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::Remote { .. } => "remote",
        }
    }
}

/// One registry entry: the display [`Origin`] plus its [`Reach`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub origin: Origin,
    pub reach: Reach,
}

impl Entry {
    /// Build an entry with `kind` derived from the reach — the two can never
    /// disagree, since every consumer reads classification from one source.
    pub fn new(name: String, current: bool, detail: String, reach: Reach) -> Self {
        let origin = Origin {
            name,
            kind: reach.label().to_string(),
            current,
            detail,
        };
        Self { origin, reach }
    }
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

    /// Entries (display origin + reach payload) in construction order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> + '_ {
        self.entries.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolve an origin by logical name, case-insensitively, returning its
    /// entry (display [`Origin`] + [`Reach`]). When the name collides across
    /// kinds — a mount alias may equal a local workspace's basename — the
    /// higher-priority kind wins, matching the lookup order that predated the
    /// registry. Collisions within one kind are an error.
    pub fn resolve(&self, name: &str) -> Result<Option<&Entry>> {
        let mut matched: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.origin.name.eq_ignore_ascii_case(name))
            .collect();
        match matched.len() {
            0 => Ok(None),
            1 => Ok(Some(matched[0])),
            _ => {
                // Remote wins a cross-kind collision; same-kind collisions stay
                // ambiguous.
                matched.sort_by_key(|entry| matches!(entry.reach, Reach::Local { .. }));
                if matches!(matched[0].reach, Reach::Local { .. })
                    == matches!(matched[1].reach, Reach::Local { .. })
                {
                    let details: Vec<&str> = matched
                        .iter()
                        .map(|entry| entry.origin.detail.as_str())
                        .collect();
                    bail!("ambiguous origin `{name}`; matches: {}", details.join(", "));
                }
                Ok(Some(matched[0]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OriginRegistry {
        OriginRegistry::new(vec![
            Entry::new(
                "sivtr".to_string(),
                true,
                "D:\\Coding\\sivtr (key1)".to_string(),
                Reach::Local {
                    root: "D:\\Coding\\sivtr".to_string(),
                },
            ),
            Entry::new(
                "desk".to_string(),
                false,
                "alice/sivtr".to_string(),
                Reach::Remote {
                    workspace_key: "key1".to_string(),
                },
            ),
        ])
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(
            Reach::Local {
                root: String::new()
            }
            .label(),
            "local"
        );
        assert_eq!(
            Reach::Remote {
                workspace_key: String::new()
            }
            .label(),
            "remote"
        );
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
            Entry::new(
                "return".to_string(),
                false,
                "D:\\a (k1)".to_string(),
                Reach::Local {
                    root: "D:\\a".to_string(),
                },
            ),
            Entry::new(
                "return".to_string(),
                false,
                "D:\\b (k2)".to_string(),
                Reach::Local {
                    root: "D:\\b".to_string(),
                },
            ),
        ]);
        let error = registry.resolve("return").expect_err("ambiguous");
        assert!(error.to_string().contains("ambiguous origin `return`"));
    }

    #[test]
    fn remote_mount_wins_over_colliding_local_workspace() {
        // A mount alias may equal a local workspace's basename; the mount
        // resolves (remote lookup ran before local workspaces pre-registry).
        let registry = OriginRegistry::new(vec![
            Entry::new(
                "proj".to_string(),
                true,
                "D:\\a\\proj (k1)".to_string(),
                Reach::Local {
                    root: "D:\\a\\proj".to_string(),
                },
            ),
            Entry::new(
                "proj".to_string(),
                false,
                "alice/proj".to_string(),
                Reach::Remote {
                    workspace_key: "k1".to_string(),
                },
            ),
        ]);
        let entry = registry.resolve("proj").expect("resolve").expect("found");
        assert!(matches!(&entry.reach, Reach::Remote { workspace_key } if workspace_key == "k1"));
    }

    #[test]
    fn resolve_returns_reach_payload_per_kind() {
        let registry = sample();
        let entry = registry.resolve("desk").expect("resolve").expect("found");
        assert!(matches!(&entry.reach, Reach::Remote { workspace_key } if workspace_key == "key1"));
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
    fn kind_serializes_as_lowercase_label() {
        let origin = sample().all().nth(1).expect("second origin").clone();
        let json = serde_json::to_string(&origin).expect("serialize origin");
        assert!(json.contains("\"kind\":\"remote\""));
        let round: Origin = serde_json::from_str(&json).expect("deserialize origin");
        assert_eq!(round, origin);
    }
}
