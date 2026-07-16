use anyhow::{bail, Result};

use crate::record::refs::normalize_scope_name;

/// Normalize a source / ref string for loading.
///
/// - `local:path` → bare `path`
/// - `Desk:terminal` → `desk:terminal` (scope case-fold)
/// - bare path / `@vars` unchanged
///
/// Remote names are mount aliases from `sivtr remote add` (like git remotes),
/// not a separate config shortcut table. Leading `&` is rejected.
pub fn expand_source(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("empty source");
    }
    // WorkSet vars and stdin markers are not scope-expanded.
    if raw.starts_with('@') {
        return Ok(raw.to_string());
    }
    if raw.starts_with('&') {
        bail!(
            "scope shortcuts (`&name`) were removed; use a remote name from `sivtr remote list` (for example `desk:agent`)"
        );
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

    let scope = resolve_scope_token(left.trim())?;
    let path = right.trim();
    match scope.as_str() {
        "local" => Ok(path.to_string()),
        name => Ok(format!("{name}:{path}")),
    }
}

/// Resolve a scope token: `local` or a normalized scope / remote name.
pub fn resolve_scope_token(token: &str) -> Result<String> {
    let token = token.trim();
    if token.is_empty() {
        bail!("empty scope");
    }
    if token.starts_with('&') {
        bail!(
            "scope shortcuts (`&name`) were removed; use a remote name from `sivtr remote list`"
        );
    }
    if token.eq_ignore_ascii_case("local") {
        return Ok("local".to_string());
    }
    normalize_scope_name(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_path_unchanged() {
        let out = expand_source("codex/abc/5").unwrap();
        assert_eq!(out, "codex/abc/5");
    }

    #[test]
    fn named_scope_normalized() {
        let out = expand_source("Desk:terminal").unwrap();
        assert_eq!(out, "desk:terminal");
    }

    #[test]
    fn local_explicit_stays_bare() {
        let out = expand_source("local:terminal/x/1").unwrap();
        assert_eq!(out, "terminal/x/1");
    }

    #[test]
    fn ampersand_shortcut_rejected() {
        assert!(expand_source("&ahs:terminal").is_err());
    }

    #[test]
    fn workset_var_passthrough() {
        let out = expand_source("@last").unwrap();
        assert_eq!(out, "@last");
    }
}
