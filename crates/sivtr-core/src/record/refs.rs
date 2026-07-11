use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::ai::AgentProvider;
use crate::record::model::WorkPartIo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRefTarget {
    Record,
    Line(usize),
    Part { io: WorkPartIo, index: usize },
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
        let (session, records) = match (self, reference.body()) {
            (
                Self::Terminal {
                    session, records, ..
                },
                WorkRefBody::Terminal { .. },
            ) => (session, records),
            (
                Self::Agent {
                    provider: None,
                    session,
                    records,
                    ..
                },
                WorkRefBody::Agent { .. },
            ) => (session, records),
            (
                Self::Agent {
                    provider: Some(expected),
                    session,
                    records,
                    ..
                },
                WorkRefBody::Agent { provider, .. },
            ) if expected == provider => (session, records),
            _ => return false,
        };

        if let Some(expected) = session.as_deref() {
            if !segment_matches(expected, reference.session()) {
                return false;
            }
        }

        if let Some(records) = records {
            if !records.contains(&reference.record_index()) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRef {
    Local(WorkRefBody),
    /// Non-current origin. Left-hand side of `origin:body`
    /// (`alias`, local workspace name, or `device/workspace`).
    Remote {
        origin: String,
        body: WorkRefBody,
    },
}

/// The local shape of a ref: a terminal record or an agent turn, plus a target
/// (record / line / part). This is what `Local` and `Remote` wrap; content
/// logic (search, match, render) operates on `WorkRefBody` and is agnostic to
/// whether the ref is local or remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRefBody {
    Terminal {
        session: String,
        record_index: usize,
        target: WorkRefTarget,
    },
    Agent {
        provider: AgentProvider,
        session: String,
        turn_index: usize,
        target: WorkRefTarget,
    },
}

impl WorkRef {
    pub fn terminal_record(session: impl Into<String>, record_index: usize) -> Self {
        Self::Local(WorkRefBody::Terminal {
            session: session.into(),
            record_index,
            target: WorkRefTarget::Record,
        })
    }

    pub fn agent_record(
        provider: AgentProvider,
        session: impl Into<String>,
        turn_index: usize,
    ) -> Self {
        Self::Local(WorkRefBody::Agent {
            provider,
            session: session.into(),
            turn_index,
            target: WorkRefTarget::Record,
        })
    }

    /// The local/remote-agnostic body: terminal record or agent turn + target.
    /// Content logic (search, match, render) should call this and match on
    /// [`WorkRefBody`] rather than on [`WorkRef`] directly.
    pub fn body(&self) -> &WorkRefBody {
        match self {
            Self::Local(body) | Self::Remote { body, .. } => body,
        }
    }

    /// Where this ref lives — `Local` or `Remote(origin)`.
    pub fn origin(&self) -> RefOrigin<'_> {
        match self {
            Self::Local(_) => RefOrigin::Local,
            Self::Remote { origin, .. } => RefOrigin::Remote(origin.as_str()),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local(_))
    }

    pub fn remote_name(&self) -> Option<&str> {
        match self {
            Self::Local(_) => None,
            Self::Remote { origin, .. } => Some(origin.as_str()),
        }
    }

    pub fn with_line(&self, line: usize) -> Self {
        self.with_target(WorkRefTarget::Line(line))
    }

    pub fn with_part(&self, io: WorkPartIo, index: usize) -> Self {
        self.with_target(WorkRefTarget::Part { io, index })
    }

    /// Replace the target (record/line/part), keeping the origin and body.
    pub fn with_target(&self, target: WorkRefTarget) -> Self {
        let body = match self.body() {
            WorkRefBody::Terminal {
                session,
                record_index,
                ..
            } => WorkRefBody::Terminal {
                session: session.clone(),
                record_index: *record_index,
                target,
            },
            WorkRefBody::Agent {
                provider,
                session,
                turn_index,
                ..
            } => WorkRefBody::Agent {
                provider: *provider,
                session: session.clone(),
                turn_index: *turn_index,
                target,
            },
        };
        self.with_body(body)
    }

    /// The record-level ref (drop line/part target), keeping the origin.
    pub fn record_ref(&self) -> Self {
        let body = match self.body() {
            WorkRefBody::Terminal {
                session,
                record_index,
                ..
            } => WorkRefBody::Terminal {
                session: session.clone(),
                record_index: *record_index,
                target: WorkRefTarget::Record,
            },
            WorkRefBody::Agent {
                provider,
                session,
                turn_index,
                ..
            } => WorkRefBody::Agent {
                provider: *provider,
                session: session.clone(),
                turn_index: *turn_index,
                target: WorkRefTarget::Record,
            },
        };
        self.with_body(body)
    }

    /// Rebuild the ref with a new body, preserving the origin (Local or
    /// Remote name).
    pub fn with_body(&self, body: WorkRefBody) -> Self {
        match self {
            Self::Local(_) => Self::Local(body),
            Self::Remote { origin, .. } => Self::Remote {
                origin: origin.clone(),
                body,
            },
        }
    }

    pub fn line(&self) -> Option<usize> {
        match self.target() {
            WorkRefTarget::Record => None,
            WorkRefTarget::Line(line) => Some(line),
            WorkRefTarget::Part { .. } => None,
        }
    }

    pub fn part(&self) -> Option<(WorkPartIo, usize)> {
        match self.target() {
            WorkRefTarget::Part { io, index } => Some((io, index)),
            WorkRefTarget::Record | WorkRefTarget::Line(_) => None,
        }
    }

    pub fn target(&self) -> WorkRefTarget {
        match self.body() {
            WorkRefBody::Terminal { target, .. } | WorkRefBody::Agent { target, .. } => *target,
        }
    }

    pub fn provider(&self) -> Option<AgentProvider> {
        match self.body() {
            WorkRefBody::Terminal { .. } => None,
            WorkRefBody::Agent { provider, .. } => Some(*provider),
        }
    }

    pub fn session(&self) -> &str {
        match self.body() {
            WorkRefBody::Terminal { session, .. } | WorkRefBody::Agent { session, .. } => session,
        }
    }

    pub fn record_index(&self) -> usize {
        match self.body() {
            WorkRefBody::Terminal { record_index, .. } => *record_index,
            WorkRefBody::Agent { turn_index, .. } => *turn_index,
        }
    }
}

/// A borrowed view of where a [`WorkRef`] lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefOrigin<'a> {
    Local,
    Remote(&'a str),
}

impl fmt::Display for WorkRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Local is the shorthand: bare body, no origin prefix.
            Self::Local(body) => write_body(f, body),
            Self::Remote { origin, body } => {
                write!(f, "{origin}:")?;
                write_body(f, body)
            }
        }
    }
}

fn write_body(f: &mut fmt::Formatter<'_>, body: &WorkRefBody) -> fmt::Result {
    match body {
        WorkRefBody::Terminal {
            session,
            record_index,
            target,
        } => write_parts(
            f,
            &["terminal", session, &record_index.to_string()],
            *target,
        ),
        WorkRefBody::Agent {
            provider,
            session,
            turn_index,
            target,
        } => write_parts(
            f,
            &[provider.command_name(), session, &turn_index.to_string()],
            *target,
        ),
    }
}

fn write_parts(f: &mut fmt::Formatter<'_>, parts: &[&str], target: WorkRefTarget) -> fmt::Result {
    write!(f, "{}", parts.join("/"))?;
    match target {
        WorkRefTarget::Record => {}
        WorkRefTarget::Line(line) => write!(f, "/{line}")?,
        WorkRefTarget::Part { io, index } => {
            write!(f, "/{}/{index}", part_segment(io))?;
        }
    }
    Ok(())
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

impl FromStr for WorkRefBody {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let parts = value
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if !(3..=5).contains(&parts.len()) {
            bail!(
                "Invalid work ref `{value}`; expected terminal/<session>/<record>[/line|/i/<part>|/o/<part>] or <provider>/<session>/<turn>[/line|/i/<part>|/o/<part>]"
            );
        }

        let target = match parts.len() {
            3 => WorkRefTarget::Record,
            4 => WorkRefTarget::Line(parse_one_based(parts[3], "line", value)?),
            5 => WorkRefTarget::Part {
                io: parse_part_io(parts[3], value)?,
                index: parse_one_based(parts[4], "part", value)?,
            },
            _ => unreachable!("length already validated"),
        };
        let item_index = parse_one_based(parts[2], "record", value)?;

        if parts[0].eq_ignore_ascii_case("terminal") {
            return Ok(Self::Terminal {
                session: parts[1].to_string(),
                record_index: item_index,
                target,
            });
        }

        let provider = AgentProvider::from_command_name(parts[0]).ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid work ref `{value}`; unknown provider `{}`",
                parts[0]
            )
        })?;
        Ok(Self::Agent {
            provider,
            session: parts[1].to_string(),
            turn_index: item_index,
            target,
        })
    }
}

impl FromStr for WorkRef {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        // `origin:body` — origin is alias, local workspace, or `device/workspace`.
        // A bare ref with no `:` is local (current workspace).
        if let Some((origin, rest)) = value.split_once(':') {
            if rest.is_empty() {
                bail!("Invalid work ref `{value}`; missing body after `:`");
            }
            reject_legacy_scheme_syntax(value)?;
            let origin = origin.trim();
            if origin.eq_ignore_ascii_case("local") {
                return Ok(Self::Local(rest.parse::<WorkRefBody>()?));
            }
            let origin = validate_origin(origin, value)?;
            return Ok(Self::Remote {
                origin,
                body: rest.parse::<WorkRefBody>()?,
            });
        }

        Ok(Self::Local(value.parse::<WorkRefBody>()?))
    }
}

/// Reject legacy `scheme://body` so it fails clearly instead of parsing as
/// `origin=scheme/` + `body=/...`.
pub fn reject_legacy_scheme_syntax(value: &str) -> Result<()> {
    if let Some((_, rest)) = value.split_once(':') {
        if rest.starts_with('/') {
            bail!(
                "Invalid work ref `{value}`; use `origin:body` (for example `desk:terminal/session/1`), not `://`"
            );
        }
    }
    Ok(())
}

/// Origin rules: `name` or `device/workspace`, each segment `[A-Za-z0-9_-]+`,
/// case-insensitive (normalized to lowercase). `local` is reserved.
fn validate_origin(name: &str, reference: &str) -> Result<String> {
    if name.is_empty() {
        bail!("Invalid work ref `{reference}`; empty origin before `:`");
    }
    let segments: Vec<&str> = name.split('/').collect();
    if segments.is_empty() || segments.len() > 2 {
        bail!("Invalid work ref `{reference}`; origin must be `name` or `device/workspace`");
    }
    let mut normalized = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.is_empty() {
            bail!("Invalid work ref `{reference}`; empty origin segment");
        }
        if segment.eq_ignore_ascii_case("local") {
            bail!(
                "Invalid work ref `{reference}`; `local` is reserved — a bare ref is already local"
            );
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            bail!(
                "Invalid work ref `{reference}`; origin segment `{segment}` must be [a-zA-Z0-9_-]+"
            );
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
            WorkRef::Local(WorkRefBody::Terminal {
                session: "current".to_string(),
                record_index: 3,
                target: WorkRefTarget::Line(12),
            })
        );
        assert_eq!(reference.to_string(), "terminal/current/3/12");
        assert_eq!(reference.record_ref().to_string(), "terminal/current/3");
    }

    #[test]
    fn parses_and_renders_terminal_part_refs() {
        let reference: WorkRef = "terminal/current/3/o/2".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef::Local(WorkRefBody::Terminal {
                session: "current".to_string(),
                record_index: 3,
                target: WorkRefTarget::Part {
                    io: WorkPartIo::Output,
                    index: 2,
                },
            })
        );
        assert_eq!(reference.part(), Some((WorkPartIo::Output, 2)));
        assert_eq!(reference.to_string(), "terminal/current/3/o/2");
        assert_eq!(reference.record_ref().to_string(), "terminal/current/3");
    }

    #[test]
    fn parses_and_renders_agent_refs() {
        let reference: WorkRef = "pi/abcdef12/2".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef::Local(WorkRefBody::Agent {
                provider: AgentProvider::Pi,
                session: "abcdef12".to_string(),
                turn_index: 2,
                target: WorkRefTarget::Record,
            })
        );
        assert_eq!(reference.with_line(7).to_string(), "pi/abcdef12/2/7");
        assert_eq!(
            reference.with_part(WorkPartIo::Input, 3).to_string(),
            "pi/abcdef12/2/i/3"
        );
    }

    #[test]
    fn local_scheme_is_shorthand_for_local() {
        // `local:body` parses to the same Local variant as a bare ref, but
        // renders bare — so bare and local: round-trip equal while being
        // distinct input strings.
        let bare: WorkRef = "terminal/session_42/3".parse().unwrap();
        let explicit: WorkRef = "local:terminal/session_42/3".parse().unwrap();
        assert_eq!(bare, explicit);
        assert!(bare.is_local());
        assert_eq!(bare.remote_name(), None);
        assert_eq!(bare.to_string(), "terminal/session_42/3");
    }

    #[test]
    fn remote_origin_parses_and_renders() {
        let reference: WorkRef = "desk:terminal/session_42/3/o/1".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef::Remote {
                origin: "desk".to_string(),
                body: WorkRefBody::Terminal {
                    session: "session_42".to_string(),
                    record_index: 3,
                    target: WorkRefTarget::Part {
                        io: WorkPartIo::Output,
                        index: 1,
                    },
                },
            }
        );
        assert!(!reference.is_local());
        assert_eq!(reference.remote_name(), Some("desk"));
        assert_eq!(reference.to_string(), "desk:terminal/session_42/3/o/1");
    }

    #[test]
    fn device_workspace_origin_parses_and_renders() {
        let reference: WorkRef = "alice/sivtr:codex/abc123/5".parse().unwrap();
        assert_eq!(reference.remote_name(), Some("alice/sivtr"));
        assert_eq!(reference.to_string(), "alice/sivtr:codex/abc123/5");
    }

    #[test]
    fn remote_origin_preserved_through_target_changes() {
        let reference: WorkRef = "laptop:codex/abc123/5".parse().unwrap();
        // with_line / record_ref must keep the origin, not drop to Local.
        assert_eq!(
            reference.with_line(2).to_string(),
            "laptop:codex/abc123/5/2"
        );
        assert_eq!(reference.record_ref().to_string(), "laptop:codex/abc123/5");
    }

    #[test]
    fn remote_refs_serialize_as_display_form() {
        let reference = WorkRef::Remote {
            origin: "desk".to_string(),
            body: WorkRefBody::Terminal {
                session: "session_42".to_string(),
                record_index: 3,
                target: WorkRefTarget::Record,
            },
        };
        assert_eq!(reference.to_string(), "desk:terminal/session_42/3");
        let json = serde_json::to_string(&reference).unwrap();
        assert_eq!(json, "\"desk:terminal/session_42/3\"");
        let restored: WorkRef = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, reference);
    }

    #[test]
    fn remote_origin_uppercased_normalized_to_lowercase() {
        let reference: WorkRef = "Desk:terminal/session_42/3".parse().unwrap();
        assert_eq!(reference.remote_name(), Some("desk"));
        assert_eq!(reference.to_string(), "desk:terminal/session_42/3");

        let full: WorkRef = "Alice/Sivtr:terminal/session_42/3".parse().unwrap();
        assert_eq!(full.remote_name(), Some("alice/sivtr"));
        assert_eq!(full.to_string(), "alice/sivtr:terminal/session_42/3");
    }

    #[test]
    fn rejects_invalid_remote_origins() {
        // digits, underscore, hyphen are valid origin segment chars.
        assert!("dev_2:terminal/x/1".parse::<WorkRef>().is_ok());
        assert!("my-box:terminal/x/1".parse::<WorkRef>().is_ok());
        assert!("alice/sivtr:terminal/x/1".parse::<WorkRef>().is_ok());
        // uppercase normalized to lowercase.
        assert_eq!(
            "Dev_2:terminal/x/1"
                .parse::<WorkRef>()
                .unwrap()
                .remote_name(),
            Some("dev_2")
        );
        // spaces / symbols / empty / legacy :// / too many segments are rejected.
        assert!("bad alias:terminal/x/1".parse::<WorkRef>().is_err());
        assert!("dev!:terminal/x/1".parse::<WorkRef>().is_err());
        assert!(":terminal/x/1".parse::<WorkRef>().is_err());
        assert!("desk://terminal/x/1".parse::<WorkRef>().is_err());
        assert!("a/b/c:terminal/x/1".parse::<WorkRef>().is_err());
        assert!("local:terminal/x/1".parse::<WorkRef>().is_ok()); // reserved -> Local
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
