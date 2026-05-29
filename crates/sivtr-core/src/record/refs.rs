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
        parts: Option<PartRangeSelector>,
    },
    Agent {
        provider: Option<AgentProvider>,
        session: Option<String>,
        records: Option<Vec<usize>>,
        lines: Option<Vec<usize>>,
        parts: Option<PartRangeSelector>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRangeSelector {
    pub io: WorkPartIo,
    pub indices: Vec<usize>,
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
        let (session, records) = match (self, reference) {
            (
                Self::Terminal {
                    session, records, ..
                },
                WorkRef::Terminal { .. },
            ) => (session, records),
            (
                Self::Agent {
                    provider: None,
                    session,
                    records,
                    ..
                },
                WorkRef::Agent { .. },
            ) => (session, records),
            (
                Self::Agent {
                    provider: Some(expected),
                    session,
                    records,
                    ..
                },
                WorkRef::Agent { provider, .. },
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

    pub fn selected_parts(&self) -> Option<&PartRangeSelector> {
        match self {
            Self::Terminal { parts, .. } | Self::Agent { parts, .. } => parts.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRef {
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
        Self::Terminal {
            session: session.into(),
            record_index,
            target: WorkRefTarget::Record,
        }
    }

    pub fn agent_record(
        provider: AgentProvider,
        session: impl Into<String>,
        turn_index: usize,
    ) -> Self {
        Self::Agent {
            provider,
            session: session.into(),
            turn_index,
            target: WorkRefTarget::Record,
        }
    }

    pub fn with_line(&self, line: usize) -> Self {
        self.with_target(WorkRefTarget::Line(line))
    }

    pub fn with_part(&self, io: WorkPartIo, index: usize) -> Self {
        self.with_target(WorkRefTarget::Part { io, index })
    }

    pub fn with_target(&self, target: WorkRefTarget) -> Self {
        match self {
            Self::Terminal {
                session,
                record_index,
                ..
            } => Self::Terminal {
                session: session.clone(),
                record_index: *record_index,
                target,
            },
            Self::Agent {
                provider,
                session,
                turn_index,
                ..
            } => Self::Agent {
                provider: *provider,
                session: session.clone(),
                turn_index: *turn_index,
                target,
            },
        }
    }

    pub fn record_ref(&self) -> Self {
        match self {
            Self::Terminal {
                session,
                record_index,
                ..
            } => Self::terminal_record(session.clone(), *record_index),
            Self::Agent {
                provider,
                session,
                turn_index,
                ..
            } => Self::agent_record(*provider, session.clone(), *turn_index),
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
        match self {
            Self::Terminal { target, .. } | Self::Agent { target, .. } => *target,
        }
    }

    pub fn provider(&self) -> Option<AgentProvider> {
        match self {
            Self::Terminal { .. } => None,
            Self::Agent { provider, .. } => Some(*provider),
        }
    }

    pub fn session(&self) -> &str {
        match self {
            Self::Terminal { session, .. } | Self::Agent { session, .. } => session,
        }
    }

    pub fn record_index(&self) -> usize {
        match self {
            Self::Terminal { record_index, .. } => *record_index,
            Self::Agent { turn_index, .. } => *turn_index,
        }
    }
}

impl fmt::Display for WorkRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Terminal {
                session,
                record_index,
                target,
            } => write_parts(
                f,
                &["terminal", session, &record_index.to_string()],
                *target,
            ),
            Self::Agent {
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
        if parts.is_empty() || parts.len() > 5 {
            bail!("Invalid work ref selector `{value}`; expected terminal[/<session>[/<record>[/line]]], <provider>[/<session>[/<turn>[/line]]], or <provider>/<session>/<turn>/<i|o>/<part-range>");
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

        let (lines, part_selector) = if parts.len() == 5 {
            let io = parse_part_io(parts[3], value)?;
            let indices = parse_index_selector(parts[4], "part", value)?;
            (None, Some(PartRangeSelector { io, indices }))
        } else {
            let lines = parts
                .get(3)
                .filter(|part| **part != "*")
                .map(|part| parse_index_selector(part, "line", value))
                .transpose()?;
            (lines, None)
        };

        let selector = if parts[0].eq_ignore_ascii_case("terminal") {
            WorkRefSelector::Terminal {
                session,
                records,
                lines,
                parts: part_selector,
            }
        } else if parts[0].eq_ignore_ascii_case("agent") {
            WorkRefSelector::Agent {
                provider: None,
                session,
                records,
                lines,
                parts: part_selector,
            }
        } else if let Some(provider) = AgentProvider::from_command_name(parts[0]) {
            WorkRefSelector::Agent {
                provider: Some(provider),
                session,
                records,
                lines,
                parts: part_selector,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkLinkKind {
    CausedBy,
    FollowsUp,
    References,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkLink {
    pub from: WorkRef,
    pub to: WorkRef,
    pub kind: WorkLinkKind,
}

impl WorkLink {
    pub fn new(from: WorkRef, to: WorkRef, kind: WorkLinkKind) -> Self {
        Self { from, to, kind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders_terminal_refs() {
        let reference: WorkRef = "terminal/current/3/12".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef::Terminal {
                session: "current".to_string(),
                record_index: 3,
                target: WorkRefTarget::Line(12),
            }
        );
        assert_eq!(reference.to_string(), "terminal/current/3/12");
        assert_eq!(reference.record_ref().to_string(), "terminal/current/3");
    }

    #[test]
    fn parses_and_renders_terminal_part_refs() {
        let reference: WorkRef = "terminal/current/3/o/2".parse().unwrap();
        assert_eq!(
            reference,
            WorkRef::Terminal {
                session: "current".to_string(),
                record_index: 3,
                target: WorkRefTarget::Part {
                    io: WorkPartIo::Output,
                    index: 2,
                },
            }
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
            WorkRef::Agent {
                provider: AgentProvider::Pi,
                session: "abcdef12".to_string(),
                turn_index: 2,
                target: WorkRefTarget::Record,
            }
        );
        assert_eq!(reference.with_line(7).to_string(), "pi/abcdef12/2/7");
        assert_eq!(
            reference.with_part(WorkPartIo::Input, 3).to_string(),
            "pi/abcdef12/2/i/3"
        );
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
                parts: None,
            }
        );
    }

    #[test]
    fn parses_part_range_selectors() {
        assert_eq!(
            "pi/abcdef12/3/o/1-3".parse::<WorkRefSelector>().unwrap(),
            WorkRefSelector::Agent {
                provider: Some(AgentProvider::Pi),
                session: Some("abcdef12".to_string()),
                records: Some(vec![3]),
                parts: Some(PartRangeSelector {
                    io: WorkPartIo::Output,
                    indices: vec![1, 2, 3],
                }),
                lines: None,
            }
        );
    }

    #[test]
    fn parses_part_selector_with_comma_list() {
        let selector: WorkRefSelector = "codex/session/1/i/2,5".parse().unwrap();
        let parts = selector.selected_parts().unwrap();
        assert_eq!(parts.io, WorkPartIo::Input);
        assert_eq!(parts.indices, vec![2, 5]);
    }

    #[test]
    fn rejects_unknown_io_in_part_selector() {
        assert!("pi/session/1/x/1-3".parse::<WorkRefSelector>().is_err());
    }

    #[test]
    fn rejects_zero_in_part_range() {
        assert!("pi/session/1/o/0-2".parse::<WorkRefSelector>().is_err());
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
