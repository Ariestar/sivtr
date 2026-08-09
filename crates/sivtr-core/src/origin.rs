//! Unified source origins.
//!
//! Every memory source — a local workspace, a remote device mount, a cloud
//! account — is described by one [`Origin`] with the same four fields, so
//! upper layers (listing, rendering, future scope resolution) never branch on
//! which kind of source they are looking at. Kind-specific details (root
//! paths, peer/share ids, cloud account) never enter [`Origin`]: the display
//! [`Origin::detail`] is composed by the source at construction time, and
//! whether a remote or cloud source happens to be ingested into local files
//! is a resolution-layer concern, not an origin concern.
//!
//! [`OriginRegistry`] is the single lookup surface: enumerate every
//! addressable origin, or resolve one by its logical name.

/// Major source category.
///
/// `#[non_exhaustive]`: adding a category (WSL, container, archive, …) only
/// requires a new variant plus a `label()` arm here; code outside this crate
/// is forced to handle the wildcard, so nothing downstream breaks.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Every origin addressable from the current context.
///
/// A pure resolution view: sources construct their own [`Origin`]s and hand
/// them in; this type owns no I/O and never interprets kind-specific fields.
#[derive(Debug, Clone, Default)]
pub struct OriginRegistry {
    origins: Vec<Origin>,
}

impl OriginRegistry {
    pub fn new(origins: Vec<Origin>) -> Self {
        Self { origins }
    }

    /// All origins in construction order.
    pub fn all(&self) -> &[Origin] {
        &self.origins
    }

    /// Resolve an origin by logical name, case-insensitively.
    pub fn resolve(&self, name: &str) -> Option<&Origin> {
        self.origins
            .iter()
            .find(|origin| origin.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OriginRegistry {
        OriginRegistry::new(vec![
            Origin {
                name: "sivtr".to_string(),
                kind: OriginKind::Local,
                current: true,
                detail: "D:\\Coding\\sivtr (key1)".to_string(),
            },
            Origin {
                name: "desk".to_string(),
                kind: OriginKind::Remote,
                current: false,
                detail: "alice/sivtr".to_string(),
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
            registry.resolve("desk").map(|o| o.name.as_str()),
            Some("desk")
        );
        assert_eq!(
            registry.resolve("DESK").map(|o| o.name.as_str()),
            Some("desk")
        );
        assert_eq!(
            registry.resolve("Sivtr").map(|o| o.name.as_str()),
            Some("sivtr")
        );
        assert_eq!(registry.resolve("missing"), None);
    }

    #[test]
    fn all_preserves_construction_order() {
        let registry = sample();
        let names: Vec<_> = registry.all().iter().map(|o| o.name.as_str()).collect();
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
}
