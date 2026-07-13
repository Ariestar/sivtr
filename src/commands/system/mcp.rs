use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use sivtr_core::ai::AgentProvider;

use crate::cli::{McpAction, McpCommand, McpInstallArgs, McpLocation};
use crate::mcp;
use crate::output;

const SERVER_NAME: &str = "sivtr";
const SERVER_ARGS: &[&str] = &["mcp", "serve"];

pub fn execute(command: McpCommand) -> Result<()> {
    match command.action {
        McpAction::Serve => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(mcp::serve_stdio())
        }
        McpAction::Install(args) => install(&args),
        McpAction::Uninstall(args) => uninstall(&args),
        McpAction::PrintConfig { target } => {
            let provider = parse_target(&target)?;
            print_config(provider);
            Ok(())
        }
    }
}

pub fn install(args: &McpInstallArgs) -> Result<()> {
    let targets = resolve_targets(&args.providers)?;
    if targets.is_empty() {
        bail!("no install targets resolved");
    }
    for target in targets {
        install_target(target, args.location)?;
    }
    Ok(())
}

fn uninstall(args: &McpInstallArgs) -> Result<()> {
    let targets = resolve_targets(&args.providers)?;
    if targets.is_empty() {
        bail!("no uninstall targets resolved");
    }
    for target in targets {
        uninstall_target(target, args.location)?;
    }
    Ok(())
}

fn install_target(target: AgentProvider, location: McpLocation) -> Result<()> {
    match target {
        AgentProvider::Claude => install_json(
            claude_config_path(location),
            "mcpServers",
            claude_cursor_entry(),
            target,
        ),
        AgentProvider::Cursor => install_json(
            cursor_config_path(location),
            "mcpServers",
            claude_cursor_entry(),
            target,
        ),
        AgentProvider::Codex => install_codex(location),
        AgentProvider::OpenCode => install_json(
            opencode_config_path(location),
            "mcp",
            opencode_entry(),
            target,
        ),
        AgentProvider::Pi => {
            if matches!(location, McpLocation::Local) {
                bail!("pi only supports global install currently; use --location global");
            }
            install_json(pi_config_path(), "mcpServers", pi_entry(), target)
        }
        AgentProvider::Hermes => {
            if matches!(location, McpLocation::Local) {
                bail!("hermes only supports global install currently; use --location global");
            }
            install_hermes()
        }
        AgentProvider::OpenClaw => {
            if matches!(location, McpLocation::Local) {
                bail!("openclaw only supports global install currently; use --location global");
            }
            install_openclaw()
        }
    }
}

fn uninstall_target(target: AgentProvider, location: McpLocation) -> Result<()> {
    match target {
        AgentProvider::Claude => uninstall_json(claude_config_path(location), "mcpServers", target),
        AgentProvider::Cursor => uninstall_json(cursor_config_path(location), "mcpServers", target),
        AgentProvider::Codex => uninstall_codex(location),
        AgentProvider::OpenCode => uninstall_json(opencode_config_path(location), "mcp", target),
        AgentProvider::Pi => {
            if matches!(location, McpLocation::Local) {
                bail!("pi only supports global uninstall currently; use --location global");
            }
            uninstall_json(pi_config_path(), "mcpServers", target)
        }
        AgentProvider::Hermes => {
            if matches!(location, McpLocation::Local) {
                bail!("hermes only supports global uninstall currently; use --location global");
            }
            uninstall_hermes()
        }
        AgentProvider::OpenClaw => {
            if matches!(location, McpLocation::Local) {
                bail!("openclaw only supports global uninstall currently; use --location global");
            }
            uninstall_openclaw()
        }
    }
}

fn resolve_targets(providers: &[String]) -> Result<Vec<AgentProvider>> {
    // Default (no -p): detect installed hosts.
    if providers.is_empty() {
        return Ok(detect_targets());
    }

    let mut out = Vec::new();
    for raw in providers {
        for part in raw.split(',') {
            let part = part.trim().to_ascii_lowercase();
            if part.is_empty() {
                continue;
            }
            if part == "auto" {
                bail!(
                    "`auto` was removed; omit `-p` to detect installed hosts, or pass `-p all` / `-p claude,cursor,...`"
                );
            }
            if part == "all" {
                return Ok(AgentProvider::all()
                    .iter()
                    .map(|spec| spec.provider)
                    .collect());
            }
            out.push(parse_target(&part)?);
        }
    }
    // Dedup while preserving order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|provider| seen.insert(*provider));
    Ok(out)
}

fn parse_target(value: &str) -> Result<AgentProvider> {
    AgentProvider::from_command_name(value).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown MCP target `{value}`; expected {}",
            valid_target_list()
        )
    })
}

fn valid_target_list() -> String {
    AgentProvider::all()
        .iter()
        .map(|spec| spec.command_name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn detect_targets() -> Vec<AgentProvider> {
    let mut targets = Vec::new();
    for spec in AgentProvider::all() {
        if provider_config_exists(spec.provider) {
            targets.push(spec.provider);
        }
    }
    if targets.is_empty() {
        targets.push(AgentProvider::Claude);
    }
    targets
}

fn provider_config_exists(provider: AgentProvider) -> bool {
    match provider {
        AgentProvider::Claude => {
            claude_config_path(McpLocation::Global).exists()
                || dirs::home_dir().is_some_and(|home| home.join(".claude").exists())
        }
        AgentProvider::Cursor => {
            cursor_config_path(McpLocation::Global).exists()
                || dirs::home_dir().is_some_and(|home| home.join(".cursor").exists())
        }
        AgentProvider::Codex => codex_config_path().exists(),
        AgentProvider::OpenCode => {
            opencode_config_path(McpLocation::Global).exists()
                || node_config_dir().join("opencode").exists()
        }
        AgentProvider::Pi => pi_config_path().exists() || pi_home().exists(),
        AgentProvider::Hermes => hermes_config_path().exists() || hermes_home().exists(),
        AgentProvider::OpenClaw => openclaw_config_path().exists() || openclaw_home().exists(),
    }
}

fn print_config(target: AgentProvider) {
    match target {
        AgentProvider::Claude => print_json_config(
            claude_config_path(McpLocation::Global),
            "mcpServers",
            claude_cursor_entry(),
        ),
        AgentProvider::Cursor => print_json_config(
            cursor_config_path(McpLocation::Global),
            "mcpServers",
            claude_cursor_entry(),
        ),
        AgentProvider::Codex => {
            let path = codex_config_path();
            output::info(format!("Add to {}", path.display()));
            println!();
            println!("{}", codex_toml_snippet());
        }
        AgentProvider::OpenCode => print_json_config(
            opencode_config_path(McpLocation::Global),
            "mcp",
            opencode_entry(),
        ),
        AgentProvider::Pi => print_json_config(pi_config_path(), "mcpServers", pi_entry()),
        AgentProvider::Hermes => {
            let path = hermes_config_path();
            output::info(format!("Add to {}", path.display()));
            println!();
            let mut root = serde_yaml::Mapping::new();
            let mut servers = serde_yaml::Mapping::new();
            servers.insert(
                serde_yaml::Value::String(SERVER_NAME.to_string()),
                hermes_entry(),
            );
            root.insert(
                serde_yaml::Value::String("mcp_servers".to_string()),
                serde_yaml::Value::Mapping(servers),
            );
            println!("{}", serde_yaml::to_string(&root).unwrap_or_default());
        }
        AgentProvider::OpenClaw => {
            let path = openclaw_config_path();
            output::info(format!("Add to {}", path.display()));
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&openclaw_print_config()).unwrap_or_default()
            );
        }
    }
}

fn print_json_config(path: PathBuf, key: &str, entry: Value) {
    output::info(format!("Add to {}", path.display()));
    println!();
    let mut root = Map::new();
    root.insert(key.to_string(), Value::Object(Map::new()));
    if let Some(Value::Object(servers)) = root.get_mut(key) {
        servers.insert(SERVER_NAME.to_string(), entry);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default()
    );
}

fn install_json(path: PathBuf, key: &str, entry: Value, provider: AgentProvider) -> Result<()> {
    let mut root = read_json_object(&path)?;
    let servers = ensure_object(&mut root, key)?;
    servers.insert(SERVER_NAME.to_string(), entry);
    write_json(&path, &Value::Object(root))?;
    output::success(format!(
        "installed MCP server for {} into {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

fn uninstall_json(path: PathBuf, key: &str, provider: AgentProvider) -> Result<()> {
    if !path.exists() {
        output::info(format!("no config at {}", path.display()));
        return Ok(());
    }
    let mut root = read_json_object(&path)?;
    if let Some(Value::Object(servers)) = root.get_mut(key) {
        if servers.remove(SERVER_NAME).is_some() {
            write_json(&path, &Value::Object(root))?;
            output::success(format!(
                "removed MCP server for {} from {}",
                provider.name(),
                path.display()
            ));
            return Ok(());
        }
    }
    output::info(format!("sivtr MCP was not installed in {}", path.display()));
    Ok(())
}

fn install_codex(location: McpLocation) -> Result<()> {
    let provider = AgentProvider::Codex;
    if matches!(location, McpLocation::Local) {
        bail!("codex only supports global install currently; use --location global");
    }
    let path = codex_config_path();
    let mut text = if path.exists() {
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?
    } else {
        String::new()
    };
    remove_codex_block(&mut text);
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&codex_toml_snippet());
    text.push('\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    output::success(format!(
        "installed MCP server for {} into {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

fn uninstall_codex(location: McpLocation) -> Result<()> {
    let provider = AgentProvider::Codex;
    if matches!(location, McpLocation::Local) {
        bail!("codex only supports global uninstall currently; use --location global");
    }
    let path = codex_config_path();
    if !path.exists() {
        output::info(format!("no Codex config at {}", path.display()));
        return Ok(());
    }
    let mut text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    if !remove_codex_block(&mut text) {
        output::info(format!("sivtr MCP was not installed in {}", path.display()));
        return Ok(());
    }
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    output::success(format!(
        "removed MCP server for {} from {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

fn remove_codex_block(text: &mut String) -> bool {
    let marker = "[mcp_servers.sivtr]";
    let Some(start) = text.find(marker) else {
        return false;
    };
    let after = &text[start + marker.len()..];
    let rel_end = after.find("\n[").map(|idx| idx + 1).unwrap_or(after.len());
    let end = start + marker.len() + rel_end;
    let mut begin = start;
    while begin > 0 && text.as_bytes()[begin - 1] == b'\n' {
        begin -= 1;
        if begin > 0 && text.as_bytes()[begin - 1] == b'\n' {
            break;
        }
    }
    text.replace_range(begin..end, "\n");
    true
}

fn install_hermes() -> Result<()> {
    let provider = AgentProvider::Hermes;
    let path = hermes_config_path();
    let mut root: serde_yaml::Value = if path.exists() {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if text.trim().is_empty() {
            serde_yaml::Value::Mapping(Default::default())
        } else {
            serde_yaml::from_str(&text)
                .with_context(|| format!("Failed to parse YAML at {}", path.display()))?
        }
    } else {
        serde_yaml::Value::Mapping(Default::default())
    };

    let servers = ensure_yaml_mapping(&mut root, "mcp_servers")?;
    servers.insert(
        serde_yaml::Value::String(SERVER_NAME.to_string()),
        hermes_entry(),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let text = serde_yaml::to_string(&root)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    output::success(format!(
        "installed MCP server for {} into {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

fn uninstall_hermes() -> Result<()> {
    let provider = AgentProvider::Hermes;
    let path = hermes_config_path();
    if !path.exists() {
        output::info(format!("no Hermes config at {}", path.display()));
        return Ok(());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut root: serde_yaml::Value = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse YAML at {}", path.display()))?;
    let removed = remove_yaml_server(&mut root, "mcp_servers", SERVER_NAME);
    if !removed {
        output::info(format!("sivtr MCP was not installed in {}", path.display()));
        return Ok(());
    }
    let text = serde_yaml::to_string(&root)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    output::success(format!(
        "removed MCP server for {} from {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

/// OpenClaw stores MCP servers under nested `mcp.servers` in openclaw.json.
/// See https://docs.openclaw.ai/gateway/configuration-reference
fn install_openclaw() -> Result<()> {
    let provider = AgentProvider::OpenClaw;
    let path = openclaw_config_path();
    let mut root = read_json_object(&path)?;
    let mcp = ensure_object(&mut root, "mcp")?;
    let servers = ensure_object(mcp, "servers")?;
    servers.insert(SERVER_NAME.to_string(), openclaw_entry());
    write_json(&path, &Value::Object(root))?;
    output::success(format!(
        "installed MCP server for {} into {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

fn uninstall_openclaw() -> Result<()> {
    let provider = AgentProvider::OpenClaw;
    let path = openclaw_config_path();
    if !path.exists() {
        output::info(format!("no OpenClaw config at {}", path.display()));
        return Ok(());
    }
    let mut root = read_json_object(&path)?;
    let removed = root
        .get_mut("mcp")
        .and_then(|mcp| mcp.as_object_mut())
        .and_then(|mcp| mcp.get_mut("servers"))
        .and_then(|servers| servers.as_object_mut())
        .and_then(|servers| servers.remove(SERVER_NAME))
        .is_some();
    if !removed {
        output::info(format!("sivtr MCP was not installed in {}", path.display()));
        return Ok(());
    }
    write_json(&path, &Value::Object(root))?;
    output::success(format!(
        "removed MCP server for {} from {}",
        provider.name(),
        path.display()
    ));
    Ok(())
}

fn claude_cursor_entry() -> Value {
    json!({
        "type": "stdio",
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn opencode_entry() -> Value {
    let mut command = vec!["sivtr"];
    command.extend_from_slice(SERVER_ARGS);
    json!({
        "type": "local",
        "command": command,
        "enabled": true,
    })
}

fn pi_entry() -> Value {
    json!({
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn openclaw_entry() -> Value {
    json!({
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn openclaw_print_config() -> Value {
    json!({
        "mcp": {
            "servers": {
                SERVER_NAME: openclaw_entry(),
            }
        }
    })
}

fn codex_toml_snippet() -> String {
    format!("[mcp_servers.{SERVER_NAME}]\ncommand = \"sivtr\"\nargs = [\"mcp\", \"serve\"]\n")
}

fn hermes_entry() -> serde_yaml::Value {
    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String("sivtr".to_string()),
    );
    let args = serde_yaml::Sequence::from([
        serde_yaml::Value::String("mcp".to_string()),
        serde_yaml::Value::String("serve".to_string()),
    ]);
    entry.insert(
        serde_yaml::Value::String("args".to_string()),
        serde_yaml::Value::Sequence(args),
    );
    serde_yaml::Value::Mapping(entry)
}

fn ensure_yaml_mapping<'a>(
    root: &'a mut serde_yaml::Value,
    key: &str,
) -> Result<&'a mut serde_yaml::Mapping> {
    if let serde_yaml::Value::Mapping(map) = root {
        if !map.contains_key(serde_yaml::Value::String(key.to_string())) {
            map.insert(
                serde_yaml::Value::String(key.to_string()),
                serde_yaml::Value::Mapping(Default::default()),
            );
        }
    } else {
        bail!("Hermes config root must be a YAML mapping");
    }
    match root.get_mut(serde_yaml::Value::String(key.to_string())) {
        Some(serde_yaml::Value::Mapping(map)) => Ok(map),
        Some(_) => bail!("`{key}` must be a YAML mapping"),
        None => unreachable!(),
    }
}

fn remove_yaml_server(root: &mut serde_yaml::Value, key: &str, name: &str) -> bool {
    let serde_yaml::Value::Mapping(map) = root else {
        return false;
    };
    let Some(serde_yaml::Value::Mapping(servers)) =
        map.get_mut(serde_yaml::Value::String(key.to_string()))
    else {
        return false;
    };
    servers
        .remove(serde_yaml::Value::String(name.to_string()))
        .is_some()
}

fn claude_config_path(location: McpLocation) -> PathBuf {
    match location {
        McpLocation::Global => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude.json"),
        McpLocation::Local => env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".mcp.json"),
    }
}

fn cursor_config_path(location: McpLocation) -> PathBuf {
    match location {
        McpLocation::Global => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cursor")
            .join("mcp.json"),
        McpLocation::Local => env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".cursor")
            .join("mcp.json"),
    }
}

fn codex_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("config.toml")
}

fn opencode_config_path(location: McpLocation) -> PathBuf {
    match location {
        McpLocation::Global => node_config_dir().join("opencode").join("opencode.json"),
        McpLocation::Local => env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("opencode.json"),
    }
}

fn node_config_dir() -> PathBuf {
    if let Ok(dir) = env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return path;
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
}

fn pi_home() -> PathBuf {
    if let Ok(path) = env::var("PI_HOME") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
}

fn pi_config_path() -> PathBuf {
    node_config_dir().join("mcp").join("mcp.json")
}

fn hermes_home() -> PathBuf {
    if let Ok(path) = env::var("HERMES_HOME") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        if let Some(local_data) = dirs::data_local_dir() {
            return local_data.join("hermes");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hermes")
}

fn hermes_config_path() -> PathBuf {
    hermes_home().join("config.yaml")
}

fn openclaw_home() -> PathBuf {
    sivtr_core::agents::openclaw::openclaw_home()
}

fn openclaw_config_path() -> PathBuf {
    sivtr_core::agents::openclaw::openclaw_config_path()
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse JSON at {}", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => bail!("{} must contain a JSON object", path.display()),
    }
}

fn ensure_object<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
    if !root.contains_key(key) {
        root.insert(key.to_string(), Value::Object(Map::new()));
    }
    match root.get_mut(key) {
        Some(Value::Object(map)) => Ok(map),
        Some(_) => bail!("`{key}` must be a JSON object"),
        None => unreachable!(),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{text}\n"))
        .with_context(|| format!("Failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_named_and_all_targets() {
        let named = resolve_targets(&["claude".into(), "cursor".into()]).expect("parse");
        assert_eq!(named, vec![AgentProvider::Claude, AgentProvider::Cursor]);

        let csv = resolve_targets(&["claude,cursor".into()]).expect("parse csv");
        assert_eq!(csv, vec![AgentProvider::Claude, AgentProvider::Cursor]);

        assert!(resolve_targets(&["nope".into()]).is_err());
        assert!(resolve_targets(&["auto".into()])
            .unwrap_err()
            .to_string()
            .contains("removed"));

        let all = resolve_targets(&["all".into()]).expect("parse all");
        assert_eq!(all.len(), AgentProvider::all().len());
    }

    #[test]
    fn resolves_all_targets() {
        let all = resolve_targets(&["all".into()]).expect("parse");
        assert_eq!(all.len(), AgentProvider::all().len());
    }

    #[test]
    fn removes_codex_block() {
        let mut text = String::from(
            "[mcp_servers.context7]\ncommand = \"x\"\n\n[mcp_servers.sivtr]\ncommand = \"sivtr\"\nargs = [\"mcp\", \"serve\"]\n\n[other]\na = 1\n",
        );
        assert!(remove_codex_block(&mut text));
        assert!(!text.contains("mcp_servers.sivtr"));
        assert!(text.contains("mcp_servers.context7"));
        assert!(text.contains("[other]"));
    }

    #[test]
    fn removes_hermes_server_via_yaml() {
        let text = "auth_key: xyz\nmcp_servers:\n  sivtr:\n    command: sivtr\n    args:\n      - mcp\n      - serve\n\nother: 1\n";
        let mut root: serde_yaml::Value = serde_yaml::from_str(text).expect("parse yaml");
        assert!(remove_yaml_server(&mut root, "mcp_servers", "sivtr"));
        let out = serde_yaml::to_string(&root).unwrap();
        assert!(!out.contains("sivtr"));
        assert!(out.contains("other: 1"));
        assert!(out.contains("mcp_servers:"));
    }

    #[test]
    fn removes_hermes_server_leaves_empty_mapping() {
        let text = "mcp_servers:\n  sivtr:\n    command: sivtr\n";
        let mut root: serde_yaml::Value = serde_yaml::from_str(text).expect("parse yaml");
        assert!(remove_yaml_server(&mut root, "mcp_servers", "sivtr"));
        let out = serde_yaml::to_string(&root).unwrap();
        assert!(out.contains("mcp_servers:"));
        assert!(!out.contains("sivtr"));
    }
}
