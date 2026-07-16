use anyhow::{bail, Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::config::SivtrConfig;
use sivtr_core::record::{
    normalize_alias_name, normalize_scope_name, resolve_scope_token,
};
use sivtr_core::workspace::{self, WorkContext};

use crate::cli::{AliasCommand, AliasSubcommand, ContextCommand, UseCommand};
use crate::output;

pub fn execute_alias(cmd: AliasCommand) -> Result<()> {
    match cmd.action {
        AliasSubcommand::List => list_aliases(),
        AliasSubcommand::Set { name, scope } => set_alias(&name, &scope),
        AliasSubcommand::Remove { name } => remove_alias(&name),
    }
}

pub fn execute_use(cmd: UseCommand) -> Result<()> {
    let paths = workspace::ensure_current_workspace()?
        .context("`sivtr use` requires a git workspace")?;
    let config = SivtrConfig::load().unwrap_or_default();

    if cmd.clear {
        workspace::save_context(&paths, &WorkContext::default())?;
        output::success("cleared workspace context");
        return Ok(());
    }

    let Some(spec) = cmd.spec.as_deref() else {
        bail!("usage: sivtr use <scope[:source]>  (or `sivtr use --clear`)");
    };

    let (scope_token, source) = parse_use_spec(spec)?;
    // `use ahs` accepts bare alias names; `&ahs` also works.
    let scope = resolve_use_scope(&scope_token, &config.scope.aliases)?;
    let scope = if scope == "local" {
        None
    } else {
        Some(normalize_scope_name(&scope)?)
    };

    if let Some(source) = source.as_deref() {
        validate_source_token(source)?;
    }

    // Persist resolved scope so context does not depend on alias later.
    let ctx = WorkContext {
        scope,
        source: source.map(|s| s.to_ascii_lowercase()),
    };
    workspace::save_context(&paths, &ctx)?;
    output::success(format!("context set to {}", format_context(&ctx)));
    Ok(())
}

pub fn execute_context(_cmd: ContextCommand) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let config = SivtrConfig::load().unwrap_or_default();
    let ctx = workspace::load_context_for_dir(&cwd)?;

    if ctx.is_empty() {
        output::plain("context: (none — bare local)");
    } else {
        output::detail("context", format_context(&ctx));
    }

    if config.scope.aliases.is_empty() {
        output::plain("aliases: (none)");
    } else {
        output::plain("aliases:");
        for (name, scope) in &config.scope.aliases {
            output::detail(format!("  &{name}"), scope.clone());
        }
    }
    Ok(())
}

fn list_aliases() -> Result<()> {
    let config = SivtrConfig::load().unwrap_or_default();
    if config.scope.aliases.is_empty() {
        output::plain("no scope aliases; set with `sivtr alias set <name> <scope>`");
        return Ok(());
    }
    for (name, scope) in &config.scope.aliases {
        output::detail(format!("&{name}"), scope.clone());
    }
    Ok(())
}

fn set_alias(name: &str, scope: &str) -> Result<()> {
    let name = normalize_alias_name(name)?;
    let scope = normalize_scope_name(scope)?;
    let mut config = SivtrConfig::load().unwrap_or_default();
    config.scope.aliases.insert(name.clone(), scope.clone());
    config.save()?;
    output::success(format!("alias `&{name}` → `{scope}`"));
    Ok(())
}

fn remove_alias(name: &str) -> Result<()> {
    let name = normalize_alias_name(name)?;
    let mut config = SivtrConfig::load().unwrap_or_default();
    if config.scope.aliases.remove(&name).is_none() {
        bail!("unknown alias `&{name}`");
    }
    config.save()?;
    output::success(format!("removed alias `&{name}`"));
    Ok(())
}

fn parse_use_spec(spec: &str) -> Result<(String, Option<String>)> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("empty use target");
    }
    if let Some((scope, source)) = spec.split_once(':') {
        let source = source.trim();
        if source.is_empty() {
            bail!("missing source after `:` in `{spec}`");
        }
        if source.contains('/') || source.contains(':') {
            bail!("context source must be a single token (terminal, agent, or provider name)");
        }
        Ok((scope.trim().to_string(), Some(source.to_string())))
    } else {
        Ok((spec.to_string(), None))
    }
}

/// Resolve scope for `sivtr use`: bare alias names, `&alias`, or a full scope.
fn resolve_use_scope(
    token: &str,
    aliases: &std::collections::BTreeMap<String, String>,
) -> Result<String> {
    let token = token.trim();
    if token.starts_with('&') {
        return resolve_scope_token(token, aliases);
    }
    if token.eq_ignore_ascii_case("local") {
        return Ok("local".to_string());
    }
    // Prefer configured alias when the bare name matches (docs: `sivtr use ahs`).
    let key = token.to_ascii_lowercase();
    if let Some(scope) = aliases.get(&key) {
        return Ok(scope.clone());
    }
    normalize_scope_name(token)
}

fn validate_source_token(source: &str) -> Result<()> {
    if source.eq_ignore_ascii_case("terminal")
        || source.eq_ignore_ascii_case("agent")
        || AgentProvider::from_command_name(source).is_some()
    {
        return Ok(());
    }
    bail!(
        "invalid source `{source}`; expected terminal, agent, or a provider name ({})",
        AgentProvider::command_names_csv()
    )
}

fn format_context(ctx: &WorkContext) -> String {
    match (
        ctx.scope.as_deref().filter(|s| !s.is_empty()),
        ctx.source.as_deref().filter(|s| !s.is_empty()),
    ) {
        (Some(scope), Some(source)) => format!("{scope}:{source}"),
        (Some(scope), None) => scope.to_string(),
        (None, Some(source)) => format!("local:{source}"),
        (None, None) => "(none)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::expand_source;
    use std::collections::BTreeMap;

    #[test]
    fn use_spec_splits_scope_and_source() {
        let (scope, source) = parse_use_spec("&ahs:claude").unwrap();
        assert_eq!(scope, "&ahs");
        assert_eq!(source.as_deref(), Some("claude"));
    }

    #[test]
    fn expand_uses_alias_table() {
        let aliases = BTreeMap::from([("ahs".to_string(), "desktop/study".to_string())]);
        let out = expand_source("&ahs:terminal", &aliases, &WorkContext::default()).unwrap();
        assert_eq!(out, "desktop/study:terminal");
    }
}
