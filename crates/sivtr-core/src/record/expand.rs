use anyhow::{bail, Result};
use std::collections::BTreeMap;

use crate::ai::AgentProvider;
use crate::record::refs::normalize_scope_name;
use crate::workspace::WorkContext;

/// Expand a source / ref string with `&alias` and workspace context defaults.
///
/// Priority:
/// 1. Explicit scope (including `&alias` and `local`)
/// 2. Current workspace context scope
/// 3. Bare local (no scope prefix)
///
/// When context has a default source and the path has no source segment,
/// the default source is prepended (`s123/3` → `claude/s123/3`).
pub fn expand_source(
    raw: &str,
    aliases: &BTreeMap<String, String>,
    ctx: &WorkContext,
) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("empty source");
    }
    // WorkSet vars and stdin markers are not scope-expanded.
    if raw.starts_with('@') {
        return Ok(raw.to_string());
    }

    let (scope_token, path_raw) = if let Some((left, right)) = raw.split_once(':') {
        if right.is_empty() {
            bail!("source `{raw}` is missing a path after `:`");
        }
        if right.starts_with('/') {
            bail!(
                "Invalid source `{raw}`; use `scope:path` (for example `desk:terminal`), not `://`"
            );
        }
        (Some(left.trim()), right.trim())
    } else {
        (None, raw)
    };

    let scope = match scope_token {
        Some(token) => Some(resolve_scope_token(token, aliases)?),
        None => ctx
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };

    let path = apply_default_source(path_raw, ctx)?;

    match scope.as_deref() {
        None | Some("local") => Ok(path),
        Some(name) => Ok(format!("{name}:{path}")),
    }
}

/// Resolve a scope token: `&alias`, `local`, or a normalized scope name.
pub fn resolve_scope_token(token: &str, aliases: &BTreeMap<String, String>) -> Result<String> {
    let token = token.trim();
    if token.is_empty() {
        bail!("empty scope");
    }
    if let Some(name) = token.strip_prefix('&') {
        return resolve_alias(name, aliases);
    }
    if token.eq_ignore_ascii_case("local") {
        return Ok("local".to_string());
    }
    normalize_scope_name(token)
}

/// Resolve `&name` / `name` against the alias table. Returns the full scope string.
pub fn resolve_alias(name: &str, aliases: &BTreeMap<String, String>) -> Result<String> {
    let name = name.trim().trim_start_matches('&').to_ascii_lowercase();
    if name.is_empty() {
        bail!("empty alias name");
    }
    aliases
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown alias `&{name}`; set with `sivtr alias set {name} <scope>`"))
}

/// Validate and normalize an alias name (no `&`, no `/`).
pub fn normalize_alias_name(name: &str) -> Result<String> {
    let name = name.trim().trim_start_matches('&');
    if name.is_empty() {
        bail!("empty alias name");
    }
    if name.contains('/') {
        bail!("alias name must be a single segment (no `/`)");
    }
    if name.eq_ignore_ascii_case("local") {
        bail!("`local` is reserved and cannot be an alias");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("alias name `{name}` must be [a-zA-Z0-9_-]+");
    }
    Ok(name.to_ascii_lowercase())
}

fn apply_default_source(path: &str, ctx: &WorkContext) -> Result<String> {
    let path = path.trim();
    if path.is_empty() {
        bail!("empty path");
    }
    if has_source_segment(path) {
        return Ok(path.to_string());
    }
    let Some(source) = ctx.source.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(path.to_string());
    };
    if !is_source_token(source) {
        bail!(
            "invalid context source `{source}`; expected terminal, agent, or a provider name"
        );
    }
    Ok(format!("{source}/{path}"))
}

fn has_source_segment(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or(path);
    is_source_token(first)
}

fn is_source_token(value: &str) -> bool {
    value.eq_ignore_ascii_case("terminal")
        || value.eq_ignore_ascii_case("agent")
        || AgentProvider::from_command_name(value).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("ahs".to_string(), "desktop/ai-help-study".to_string()),
            ("desk".to_string(), "desk".to_string()),
        ])
    }

    #[test]
    fn expands_ampersand_alias() {
        let out = expand_source("&ahs:codex/4", &aliases(), &WorkContext::default()).unwrap();
        assert_eq!(out, "desktop/ai-help-study:codex/4");
    }

    #[test]
    fn expands_context_scope_for_bare_path() {
        let ctx = WorkContext {
            scope: Some("desk".to_string()),
            source: None,
        };
        let out = expand_source("codex/abc/5", &aliases(), &ctx).unwrap();
        assert_eq!(out, "desk:codex/abc/5");
    }

    #[test]
    fn expands_context_source_for_short_path() {
        let ctx = WorkContext {
            scope: Some("desktop/ai-help-study".to_string()),
            source: Some("claude".to_string()),
        };
        let out = expand_source("s123/3", &aliases(), &ctx).unwrap();
        assert_eq!(out, "desktop/ai-help-study:claude/s123/3");
    }

    #[test]
    fn explicit_scope_wins_over_context() {
        let ctx = WorkContext {
            scope: Some("desk".to_string()),
            source: Some("claude".to_string()),
        };
        let out = expand_source("docs:terminal", &aliases(), &ctx).unwrap();
        assert_eq!(out, "docs:terminal");
    }

    #[test]
    fn local_explicit_stays_bare() {
        let out = expand_source("local:terminal/x/1", &aliases(), &WorkContext::default()).unwrap();
        assert_eq!(out, "terminal/x/1");
    }

    #[test]
    fn unknown_alias_errors() {
        assert!(expand_source("&nope:terminal", &aliases(), &WorkContext::default()).is_err());
    }

    #[test]
    fn workset_var_passthrough() {
        let out = expand_source("@last", &aliases(), &WorkContext::default()).unwrap();
        assert_eq!(out, "@last");
    }
}
