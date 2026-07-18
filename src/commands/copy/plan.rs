//! Copy plan: one address model for terminal and agent sources.

use sivtr_core::record::WorkAt;

use crate::commands::select::CommandSelection;

/// Relative dialogue selection (1 = newest). Same axis as historical block selectors.
pub type DialogueSelect = CommandSelection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Both,
    Input,
    Output,
    Command,
    /// Address already pinned a part/line; take that content only.
    Exact(WorkAt),
}

#[derive(Clone, Debug, Default)]
pub struct CopyFilters {
    pub print: bool,
    pub ansi: bool,
    pub regex: Option<String>,
    pub lines: Option<String>,
    pub prompt: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
pub struct CopyPlan {
    /// Source / work-ref token. `None` = current terminal session.
    pub address: Option<String>,
    /// Used only when `address` does not already pin a record.
    pub dialogues: DialogueSelect,
    pub projection: Projection,
    pub pick: bool,
    pub filters: CopyFilters,
}

/// True when a free token is relative dialogue selection, not an address.
pub fn is_dialogues_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    if let Some((a, b)) = token.split_once("..") {
        return is_positive_int(a) && is_positive_int(b);
    }
    is_positive_int(token)
}

fn is_positive_int(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Parse 0..=2 free tokens into address + dialogues.
///
/// Grammar: `copy [address] [dialogues]`
/// - dialogues form: `N` or `A..B` only
/// - everything else is address (validated later by source/ref parse)
pub fn parse_address_dialogues(
    tokens: &[String],
) -> Result<(Option<String>, DialogueSelect), String> {
    let tokens: Vec<&str> = tokens
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();

    match tokens.as_slice() {
        [] => Ok((None, DialogueSelect::RecentSingle(1))),
        [only] if is_dialogues_token(only) => {
            let select = crate::commands::select::parse_selector(only)
                .map_err(|e| e.to_string())?;
            Ok((None, select))
        }
        [address] => Ok((Some((*address).to_string()), DialogueSelect::RecentSingle(1))),
        [address, dialogues] => {
            if !is_dialogues_token(dialogues) {
                return Err(format!(
                    "second argument `{dialogues}` is not a dialogue selector; use `N` or `A..B` (address first: `copy <address> <dialogues>`)"
                ));
            }
            let select = crate::commands::select::parse_selector(dialogues)
                .map_err(|e| e.to_string())?;
            Ok((Some((*address).to_string()), select))
        }
        _ => Err(
            "too many arguments; expected `copy [address] [dialogues]` (at most two positionals)"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogues_token_detects_relative_selectors() {
        assert!(is_dialogues_token("1"));
        assert!(is_dialogues_token("12"));
        assert!(is_dialogues_token("2..5"));
        assert!(!is_dialogues_token("codex"));
        assert!(!is_dialogues_token("codex/abc"));
        assert!(!is_dialogues_token("terminal/s/12"));
        assert!(!is_dialogues_token("2.."));
        assert!(!is_dialogues_token(""));
    }

    #[test]
    fn parse_tokens_address_then_dialogues() {
        let (addr, sel) = parse_address_dialogues(&[]).unwrap();
        assert!(addr.is_none());
        assert_eq!(sel, DialogueSelect::RecentSingle(1));

        let (addr, sel) = parse_address_dialogues(&["3".into()]).unwrap();
        assert!(addr.is_none());
        assert_eq!(sel, DialogueSelect::RecentSingle(3));

        let (addr, sel) = parse_address_dialogues(&["codex".into()]).unwrap();
        assert_eq!(addr.as_deref(), Some("codex"));
        assert_eq!(sel, DialogueSelect::RecentSingle(1));

        let (addr, sel) = parse_address_dialogues(&["codex".into(), "2..4".into()]).unwrap();
        assert_eq!(addr.as_deref(), Some("codex"));
        assert_eq!(
            sel,
            DialogueSelect::RecentRange {
                newer: 2,
                older: 4
            }
        );
    }

    #[test]
    fn parse_tokens_rejects_dialogues_before_address() {
        let err = parse_address_dialogues(&["3".into(), "codex".into()]).unwrap_err();
        assert!(err.contains("dialogue selector") || err.contains("second argument"));
    }
}
