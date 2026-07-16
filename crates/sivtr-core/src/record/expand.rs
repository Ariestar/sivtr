use anyhow::{bail, Result};
use std::collections::BTreeMap;

use crate::record::refs::normalize_scope_name;

/// Expand a source / ref string: resolve `&alias` on the scope side only.
///
/// - `&ahs:codex/4` → `desktop/ai-help-study:codex/4` (when configured)
/// - `local:path` → bare `path`
/// - bare path / `@vars` unchanged
pub fn expand_source(raw: &str, aliases: &BTreeMap<String, String>) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("empty source");
    }
    // WorkSet vars and stdin markers are not scope-expanded.
    if raw.starts_with('@') {
        return Ok(raw.to_string());
    }

    let Some((left, right)) = raw.split_once(':') else {
        return Ok(raw.to_string());
    };
    if right.is_empty() {
        bail!("source `{raw}` is missing a path after `:`");
    }
    if right.starts_with('/') {
        bail!("Invalid source `{raw}`; use `scope:path` (for example `desk:terminal`), not `://`");
    }

    let scope = resolve_scope_token(left.trim(), aliases)?;
    let path = right.trim();
    match scope.as_str() {
        "local" => Ok(path.to_string()),
        name => Ok(format!("{name}:{path}")),
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
    aliases.get(&name).cloned().ok_or_else(|| {
        anyhow::anyhow!("unknown alias `&{name}`; set with `sivtr alias set {name} <scope>`")
    })
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
        let out = expand_source("&ahs:codex/4", &aliases()).unwrap();
        assert_eq!(out, "desktop/ai-help-study:codex/4");
    }

    #[test]
    fn bare_path_unchanged() {
        let out = expand_source("codex/abc/5", &aliases()).unwrap();
        assert_eq!(out, "codex/abc/5");
    }

    #[test]
    fn named_scope_normalized() {
        let out = expand_source("Desk:terminal", &aliases()).unwrap();
        assert_eq!(out, "desk:terminal");
    }

    #[test]
    fn local_explicit_stays_bare() {
        let out = expand_source("local:terminal/x/1", &aliases()).unwrap();
        assert_eq!(out, "terminal/x/1");
    }

    #[test]
    fn unknown_alias_errors() {
        assert!(expand_source("&nope:terminal", &aliases()).is_err());
    }

    #[test]
    fn workset_var_passthrough() {
        let out = expand_source("@last", &aliases()).unwrap();
        assert_eq!(out, "@last");
    }
}
