use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::ai::AgentProvider;
use crate::record::model::WorkPartIo;

/// Where a ref lives: current workspace, or a named scope (`docs`, `desk`, `alice/sivtr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkScope {
    Local,
    Named(String),
}

impl WorkScope {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Local => None,
            Self::Named(name) => Some(name.as_str()),
        }
    }

    /// Parse a non-local scope name (`name` or `device/workspace`).
    pub fn named(raw: &str) -> Result<Self> {
        Ok(Self::Named(parse_scope_name(raw)?))
    }
}

/// Scope-local logical path: `source/session/index` (not a filesystem path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkPath {
    Terminal {
        session: String,
        index: usize,
    },
    Agent {
        provider: AgentProvider,
        session: String,
        index: usize,
    },
}

impl WorkPath {
    pub fn session(&self) -> &str {
        match self {
            Self::Terminal { session, .. } | Self::Agent { session, .. } => session,
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Self::Terminal { index, .. } | Self::Agent { index, .. } => *index,
        }
    }

    pub fn provider(&self) -> Option<AgentProvider> {
        match self {
            Self::Terminal { .. } => None,
            Self::Agent { provider, .. } => Some(*provider),
        }
    }

    pub fn with_session(&self, session: impl Into<String>) -> Self {
        match self {
            Self::Terminal { index, .. } => Self::Terminal {
                session: session.into(),
                index: *index,
            },
            Self::Agent {
                provider, index, ..
            } => Self::Agent {
                provider: *provider,
                session: session.into(),
                index: *index,
            },
        }
    }
}

impl fmt::Display for WorkPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Terminal { session, index } => {
                write!(f, "terminal/{session}/{index}")
            }
            Self::Agent {
                provider,
                session,
                index,
            } => write!(f, "{}/{session}/{index}", provider.command_name()),
        }
    }
}

/// Where on a resolved record this ref lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkAt {
    Whole,
    Line(usize),
    Part { io: WorkPartIo, index: usize },
}

/// Exact address: `[scope:]path[/at]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRef {
    pub scope: WorkScope,
    pub path: WorkPath,
    pub at: WorkAt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRefSelector {
    Terminal {
        session: Option<String>,
        records: Option<Vec<usize>>,
        lines: Option<Vec<usize>>,
    },
    Agent {
        provider: Option<AgentProvider>,
        session: Option<String>,
        records: Option<Vec<usize>>,
        lines: Option<Vec<usize>>,
    },
}

impl WorkRefSelector {
    pub fn providers(&self) -> Vec<AgentProvider> {
        match self {
            Self::Terminal { .. } => Vec::new(),
            Self::Agent {
                provider: Some(provider),
                ..
            } => vec![*provider],
            Self::Agent { provider: None, .. } => AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect(),
        }
    }

    pub fn matches_work_ref(&self, reference: &WorkRef) -> bool {
        let (session, records) = match (self, &reference.path) {
            (
                Self::Terminal {
                    session, records, ..
                },
                WorkPath::Terminal { .. },
            ) => (session, records),
            (
                Self::Agent {
                    provider: None,
                    session,
                    records,
                    ..
                },
                WorkPath::Agent { .. },
            ) => (session, records),
            (
                Self::Agent {
                    provider: Some(expected),
                    session,
                    records,
                    ..
                },
                WorkPath::Agent { provider, .. },
            ) if expected == provider => (session, records),
            _ => return false,
        };

        if let Some(expected) = session.as_deref() {
            if !segment_matches(expected, reference.session()) {
                return false;
            }
        }

        if let Some(records) = records {
            if !records.contains(&reference.index()) {
                return false;
            }
        }

        true
    }

    pub fn selected_lines(&self) -> Option<&[usize]> {
        match self {
            Self::Terminal { lines, .. } | Self::Agent { lines, .. } => lines.as_deref(),
        }
    }
}

impl WorkRef {
    pub fn terminal(session: impl Into<String>, index: usize) -> Self {
        Self {
            scope: WorkScope::Local,
            path: WorkPath::Terminal {
                session: session.into(),
                index,
            },
            at: WorkAt::Whole,
        }
    }

    pub fn agent(provider: AgentProvider, session: impl Into<String>, index: usize) -> Self {
        Self {
            scope: WorkScope::Local,
            path: WorkPath::Agent {
                provider,
                session: session.into(),
                index,
            },
            at: WorkAt::Whole,
        }
    }

    pub fn is_local(&self) -> bool {
        self.scope.is_local()
    }

    pub fn scope_name(&self) -> Option<&str> {
        self.scope.name()
    }

    pub fn with_scope(&self, scope: WorkScope) -> Self {
        Self {
            scope,
            path: self.path.clone(),
            at: self.at,
        }
    }

    pub fn with_named_scope(&self, name: impl Into<String>) -> Self {
        self.with_scope(WorkScope::Named(name.into()))
    }

    pub fn with_path(&self, path: WorkPath) -> Self {
        Self {
            scope: self.scope.clone(),
            path,
            at: self.at,
        }
    }

    pub fn with_session(&self, session: impl Into<String>) -> Self {
        self.with_path(self.path.with_session(session))
    }

    pub fn with_line(&self, line: usize) -> Self {
        self.with_at(WorkAt::Line(line))
    }

    pub fn with_part(&self, io: WorkPartIo, index: usize) -> Self {
        self.with_at(WorkAt::Part { io, index })
    }

    pub fn with_at(&self, at: WorkAt) -> Self {
        Self {
            scope: self.scope.clone(),
            path: self.path.clone(),
            at,
        }
    }

    pub fn whole(&self) -> Self {
        self.with_at(WorkAt::Whole)
    }

    pub fn line(&self) -> Option<usize> {
        match self.at {
            WorkAt::Line(line) => Some(line),
            WorkAt::Whole | WorkAt::Part { .. } => None,
        }
    }

    pub fn part(&self) -> Option<(WorkPartIo, usize)> {
        match self.at {
            WorkAt::Part { io, index } => Some((io, index)),
            WorkAt::Whole | WorkAt::Line(_) => None,
        }
    }

    pub fn provider(&self) -> Option<AgentProvider> {
        self.path.provider()
    }

    pub fn session(&self) -> &str {
        self.path.session()
    }

    pub fn index(&self) -> usize {
        self.path.index()
    }
}

impl fmt::Display for WorkRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let WorkScope::Named(name) = &self.scope {
            write!(f, "{name}:")?;
        }
        write!(f, "{}", self.path)?;
        match self.at {
            WorkAt::Whole => {}
            WorkAt::Line(line) => write!(f, "/{line}")?,
            WorkAt::Part { io, index } => write!(f, "/{}/{index}", part_segment(io))?,
        }
        Ok(())
    }
}

impl Serialize for WorkRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for WorkRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for WorkRefSelector {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let parts = value
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() || parts.len() > 4 {
            bail!("Invalid work ref selector `{value}`; expected terminal[/<session>[/<record>[/line]]], agent[/<session>[/<turn>[/line]]], or <provider>[/<session>[/<turn>[/line]]]");
        }

        let session = parts
            .get(1)
            .filter(|part| **part != "*")
            .map(|part| (*part).to_string());
        let records = parts
            .get(2)
            .filter(|part| **part != "*")
            .map(|part| parse_index_selector(part, "record", value))
            .transpose()?;
        let lines = parts
            .get(3)
            .filter(|part| **part != "*")
            .map(|part| parse_index_selector(part, "line", value))
            .transpose()?;

        let selector = if parts[0].eq_ignore_ascii_case("terminal") {
            WorkRefSelector::Terminal {
                session,
                records,
                lines,
            }
        } else if parts[0].eq_ignore_ascii_case("agent") {
            WorkRefSelector::Agent {
                provider: None,
                session,
                records,
                lines,
            }
        } else if let Some(provider) = AgentProvider::from_command_name(parts[0]) {
            WorkRefSelector::Agent {
                provider: Some(provider),
                session,
                records,
                lines,
            }
        } else {
            bail!(
                "Invalid work ref selector `{value}`; unknown source `{}`",
                parts[0]
            );
        };

        Ok(selector)
    }
}

impl FromStr for WorkRef {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        // `scope:path[/at]` — bare path is local (current workspace).
        if let Some((scope_raw, rest)) = value.split_once(':') {
            if rest.is_empty() {
                bail!("Invalid work ref `{value}`; missing path after `:`");
            }
            if rest.starts_with('/') {
                bail!(
                    "Invalid work ref `{value}`; use `scope:path` (for example `desk:terminal/session/1`), not `://`"
                );
            }
            let scope_raw = scope_raw.trim();
            let (path, at) = parse_path_and_at(rest)?;
            if scope_raw.eq_ignore_ascii_case("local") {
                return Ok(Self {
                    scope: WorkScope::Local,
                    path,
                    at,
                });
            }
            return Ok(Self {
                scope: WorkScope::Named(parse_scope_name(scope_raw)?),
                path,
                at,
            });
        }

        let (path, at) = parse_path_and_at(value)?;
        Ok(Self {
            scope: WorkScope::Local,
            path,
            at,
        })
    }
}

fn parse_path_and_at(value: &str) -> Result<(WorkPath, WorkAt)> {
    let parts = value
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if !(3..=5).contains(&parts.len()) {
        bail!(
            "Invalid work ref `{value}`; expected terminal/<session>/<index>[/line|/i/<part>|/o/<part>] or <provider>/<session>/<index>[/line|/i/<part>|/o/<part>]"
        );
    }

    let at = match parts.len() {
        3 => WorkAt::Whole,
        4 => WorkAt::Line(parse_one_based(parts[3], "line", value)?),
        5 => WorkAt::Part {
            io: parse_part_io(parts[3], value)?,
            index: parse_one_based(parts[4], "part", value)?,
        },
        _ => unreachable!("length already validated"),
    };
    let index = parse_one_based(parts[2], "index", value)?;

    if parts[0].eq_ignore_ascii_case("terminal") {
        return Ok((
            WorkPath::Terminal {
                session: parts[1].to_string(),
                index,
            },
            at,
        ));
    }

    let provider = AgentProvider::from_command_name(parts[0]).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid work ref `{value}`; unknown provider `{}`",
            parts[0]
        )
    })?;
    Ok((
        WorkPath::Agent {
            provider,
            session: parts[1].to_string(),
            index,
        },
        at,
    ))
}

/// Normalize a scope name: `name` or `device/workspace`, lowercase, `local` reserved.
pub fn normalize_scope_name(name: &str) -> Result<String> {
    parse_scope_name(name)
}

/// Scope rules: `name` or `device/workspace`, each segment `[A-Za-z0-9_-]+`,
/// case-insensitive (normalized to lowercase). `local` is reserved.
fn parse_scope_name(name: &str) -> Result<String> {
    if name.is_empty() {
        bail!("Invalid work ref; empty scope before `:`");
    }
    let segments: Vec<&str> = name.split('/').collect();
    if segments.is_empty() || segments.len() > 2 {
        bail!("Invalid work ref; scope must be `name` or `device/workspace`");
    }
    let mut normalized = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.is_empty() {
            bail!("Invalid work ref; empty scope segment");
        }
        if segment.eq_ignore_ascii_case("local") {
            bail!("Invalid work ref; `local` is reserved — a bare ref is already local");
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            bail!("Invalid work ref; scope segment `{segment}` must be [a-zA-Z0-9_-]+");
        }
        normalized.push(segment.to_ascii_lowercase());
    }
    Ok(normalized.join("/"))
}

fn parse_one_based(part: &str, label: &str, reference: &str) -> Result<usize> {
    let value = part.parse::<usize>().with_context(|| {
        format!("Invalid work ref `{reference}`; {label} index must be a positive integer")
    })?;
    if value == 0 {
        bail!("Invalid work ref `{reference}`; {label} index must be 1-based");
    }
    Ok(value)
}

fn parse_part_io(part: &str, reference: &str) -> Result<WorkPartIo> {
    match part {
        "i" => Ok(WorkPartIo::Input),
        "o" => Ok(WorkPartIo::Output),
        _ => bail!("Invalid work ref `{reference}`; expected `i` or `o` part selector"),
    }
}

fn part_segment(io: WorkPartIo) -> &'static str {
    match io {
        WorkPartIo::Input => "i",
        WorkPartIo::Output => "o",
    }
}

fn parse_index_selector(part: &str, label: &str, reference: &str) -> Result<Vec<usize>> {
    let mut indices = Vec::new();
    for raw_token in part.split(',') {
        let token = raw_token.trim();
        if token.is_empty() {
            bail!("Invalid work ref selector `{reference}`; empty {label} selector segment");
        }

        if let Some((start, end)) = token.split_once('-') {
            let start = parse_one_based(start, label, reference)?;
            let end = parse_one_based(end, label, reference)?;
            if start > end {
                bail!(
                    "Invalid work ref selector `{reference}`; {label} range start must be <= end"
                );
            }
            indices.extend(start..=end);
        } else {
            indices.push(parse_one_based(token, label, reference)?);
        }
    }

    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

fn segment_matches(expected: &str, actual: &str) -> bool {
    actual == expected || actual.starts_with(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders_terminal_refs() {
        let reference: WorkRef = "terminal/current/3/12".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef {
                scope: WorkScope::Local,
                path: WorkPath::Terminal {
                    session: "current".to_string(),
                    index: 3,
                },
                at: WorkAt::Line(12),
            }
        );
        assert_eq!(reference.to_string(), "terminal/current/3/12");
        assert_eq!(reference.whole().to_string(), "terminal/current/3");
    }

    #[test]
    fn parses_and_renders_terminal_part_refs() {
        let reference: WorkRef = "terminal/current/3/o/2".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef {
                scope: WorkScope::Local,
                path: WorkPath::Terminal {
                    session: "current".to_string(),
                    index: 3,
                },
                at: WorkAt::Part {
                    io: WorkPartIo::Output,
                    index: 2,
                },
            }
        );
        assert_eq!(reference.part(), Some((WorkPartIo::Output, 2)));
        assert_eq!(reference.to_string(), "terminal/current/3/o/2");
        assert_eq!(reference.whole().to_string(), "terminal/current/3");
    }

    #[test]
    fn parses_and_renders_agent_refs() {
        let reference: WorkRef = "pi/abcdef12/2".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef {
                scope: WorkScope::Local,
                path: WorkPath::Agent {
                    provider: AgentProvider::Pi,
                    session: "abcdef12".to_string(),
                    index: 2,
                },
                at: WorkAt::Whole,
            }
        );
        assert_eq!(reference.with_line(7).to_string(), "pi/abcdef12/2/7");
        assert_eq!(
            reference.with_part(WorkPartIo::Input, 3).to_string(),
            "pi/abcdef12/2/i/3"
        );
    }

    #[test]
    fn local_scope_is_shorthand_for_local() {
        let bare: WorkRef = "terminal/session_42/3".parse().unwrap();
        let explicit: WorkRef = "local:terminal/session_42/3".parse().unwrap();
        assert_eq!(bare, explicit);
        assert!(bare.is_local());
        assert_eq!(bare.scope_name(), None);
        assert_eq!(bare.to_string(), "terminal/session_42/3");
    }

    #[test]
    fn named_scope_parses_and_renders() {
        let reference: WorkRef = "desk:terminal/session_42/3/o/1".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef {
                scope: WorkScope::Named("desk".to_string()),
                path: WorkPath::Terminal {
                    session: "session_42".to_string(),
                    index: 3,
                },
                at: WorkAt::Part {
                    io: WorkPartIo::Output,
                    index: 1,
                },
            }
        );
        assert!(!reference.is_local());
        assert_eq!(reference.scope_name(), Some("desk"));
        assert_eq!(reference.to_string(), "desk:terminal/session_42/3/o/1");
    }

    #[test]
    fn device_workspace_scope_parses_and_renders() {
        let reference: WorkRef = "alice/sivtr:codex/abc123/5".parse().unwrap();
        assert_eq!(reference.scope_name(), Some("alice/sivtr"));
        assert_eq!(reference.to_string(), "alice/sivtr:codex/abc123/5");
    }

    #[test]
    fn scope_preserved_through_at_changes() {
        let reference: WorkRef = "laptop:codex/abc123/5".parse().unwrap();
        assert_eq!(
            reference.with_line(2).to_string(),
            "laptop:codex/abc123/5/2"
        );
        assert_eq!(reference.whole().to_string(), "laptop:codex/abc123/5");
    }

    #[test]
    fn named_refs_serialize_as_display_form() {
        let reference = WorkRef {
            scope: WorkScope::Named("desk".to_string()),
            path: WorkPath::Terminal {
                session: "session_42".to_string(),
                index: 3,
            },
            at: WorkAt::Whole,
        };
        assert_eq!(reference.to_string(), "desk:terminal/session_42/3");
        let json = serde_json::to_string(&reference).unwrap();
        assert_eq!(json, "\"desk:terminal/session_42/3\"");
        let restored: WorkRef = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, reference);
    }

    #[test]
    fn named_scope_uppercased_normalized_to_lowercase() {
        let reference: WorkRef = "Desk:terminal/session_42/3".parse().unwrap();
        assert_eq!(reference.scope_name(), Some("desk"));
        assert_eq!(reference.to_string(), "desk:terminal/session_42/3");

        let full: WorkRef = "Alice/Sivtr:terminal/session_42/3".parse().unwrap();
        assert_eq!(full.scope_name(), Some("alice/sivtr"));
        assert_eq!(full.to_string(), "alice/sivtr:terminal/session_42/3");
    }

    #[test]
    fn rejects_invalid_scopes() {
        assert!("dev_2:terminal/x/1".parse::<WorkRef>().is_ok());
        assert!("my-box:terminal/x/1".parse::<WorkRef>().is_ok());
        assert!("alice/sivtr:terminal/x/1".parse::<WorkRef>().is_ok());
        assert_eq!(
            "Dev_2:terminal/x/1"
                .parse::<WorkRef>()
                .unwrap()
                .scope_name(),
            Some("dev_2")
        );
        assert!("bad alias:terminal/x/1".parse::<WorkRef>().is_err());
        assert!("dev!:terminal/x/1".parse::<WorkRef>().is_err());
        assert!(":terminal/x/1".parse::<WorkRef>().is_err());
        assert!("desk://terminal/x/1".parse::<WorkRef>().is_err());
        assert!("a/b/c:terminal/x/1".parse::<WorkRef>().is_err());
        assert!("local:terminal/x/1".parse::<WorkRef>().is_ok());
    }

    #[test]
    fn rejects_zero_indices() {
        assert!("pi/session/0".parse::<WorkRef>().is_err());
        assert!("pi/session/1/0".parse::<WorkRef>().is_err());
        assert!("pi/session/1/i/0".parse::<WorkRef>().is_err());
    }

    #[test]
    fn rejects_unknown_part_selector() {
        assert!("pi/session/1/x/1".parse::<WorkRef>().is_err());
    }

    #[test]
    fn parses_ref_selectors() {
        assert_eq!(
            "pi/abcdef12/2-4,7/*".parse::<WorkRefSelector>().unwrap(),
            WorkRefSelector::Agent {
                provider: Some(AgentProvider::Pi),
                session: Some("abcdef12".to_string()),
                records: Some(vec![2, 3, 4, 7]),
                lines: None,
            }
        );
    }

    #[test]
    fn rejects_multi_index_concrete_refs() {
        assert!("pi/session/1-2".parse::<WorkRef>().is_err());
        assert!("agent/session/1".parse::<WorkRef>().is_err());
    }

    #[test]
    fn rejects_descending_ranges() {
        assert!("pi/session/5-3".parse::<WorkRefSelector>().is_err());
    }
}
