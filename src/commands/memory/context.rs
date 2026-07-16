use anyhow::{bail, Result};
use sivtr_core::config::SivtrConfig;
use sivtr_core::record::{normalize_alias_name, normalize_scope_name};

use crate::cli::{AliasCommand, AliasSubcommand};
use crate::output;

pub fn execute_alias(cmd: AliasCommand) -> Result<()> {
    match cmd.action {
        AliasSubcommand::List => list_aliases(),
        AliasSubcommand::Set { name, scope } => set_alias(&name, &scope),
        AliasSubcommand::Remove { name } => remove_alias(&name),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::expand_source;
    use std::collections::BTreeMap;

    #[test]
    fn expand_uses_alias_table() {
        let aliases = BTreeMap::from([("ahs".to_string(), "desktop/study".to_string())]);
        let out = expand_source("&ahs:terminal", &aliases).unwrap();
        assert_eq!(out, "desktop/study:terminal");
    }

    #[test]
    fn set_alias_normalizes_name_and_scope() {
        let name = normalize_alias_name("&Ahs").unwrap();
        let scope = normalize_scope_name("Desktop/Study").unwrap();
        assert_eq!(name, "ahs");
        assert_eq!(scope, "desktop/study");
    }
}
