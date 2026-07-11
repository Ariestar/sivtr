use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::cli::{McpAction, McpCommand, McpInstallArgs, McpLocation, McpTarget};
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
            print_config(target);
            Ok(())
        }
    }
}

fn install(args: &McpInstallArgs) -> Result<()> {
    let targets = resolve_targets(&args.target)?;
    if targets.is_empty() {
        bail!("no install targets resolved from `{}`", args.target);
    }
    for target in targets {
        match target {
            McpTarget::Claude => install_claude(args.location)?,
            McpTarget::Cursor => install_cursor(args.location)?,
            McpTarget::Codex => install_codex(args.location)?,
        }
    }
    Ok(())
}

fn uninstall(args: &McpInstallArgs) -> Result<()> {
    let targets = resolve_targets(&args.target)?;
    if targets.is_empty() {
        bail!("no uninstall targets resolved from `{}`", args.target);
    }
    for target in targets {
        match target {
            McpTarget::Claude => uninstall_claude(args.location)?,
            McpTarget::Cursor => uninstall_cursor(args.location)?,
            McpTarget::Codex => uninstall_codex(args.location)?,
        }
    }
    Ok(())
}

fn resolve_targets(raw: &str) -> Result<Vec<McpTarget>> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed == "auto" {
        return Ok(detect_targets());
    }
    if trimmed == "all" {
        return Ok(vec![McpTarget::Claude, McpTarget::Cursor, McpTarget::Codex]);
    }
    let mut out = Vec::new();
    for part in trimmed.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        out.push(match part {
            "claude" | "claude-code" | "claude_code" => McpTarget::Claude,
            "cursor" => McpTarget::Cursor,
            "codex" => McpTarget::Codex,
            other => {
                bail!("unknown MCP target `{other}`; expected claude, cursor, codex, auto, or all")
            }
        });
    }
    Ok(out)
}

fn detect_targets() -> Vec<McpTarget> {
    let mut targets = Vec::new();
    if claude_config_path(McpLocation::Global).exists()
        || dirs::home_dir().is_some_and(|home| home.join(".claude").exists())
    {
        targets.push(McpTarget::Claude);
    }
    if cursor_config_path(McpLocation::Global).exists()
        || dirs::home_dir().is_some_and(|home| home.join(".cursor").exists())
    {
        targets.push(McpTarget::Cursor);
    }
    if codex_config_path(McpLocation::Global).exists()
        || dirs::home_dir().is_some_and(|home| home.join(".codex").exists())
    {
        targets.push(McpTarget::Codex);
    }
    if targets.is_empty() {
        // Default to Claude Code when nothing is detected.
        targets.push(McpTarget::Claude);
    }
    targets
}

fn print_config(target: McpTarget) {
    match target {
        McpTarget::Claude => {
            let path = claude_config_path(McpLocation::Global);
            output::info(format!("Add to {}", path.display()));
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        SERVER_NAME: claude_server_entry()
                    }
                }))
                .unwrap_or_default()
            );
        }
        McpTarget::Cursor => {
            let path = cursor_config_path(McpLocation::Global);
            output::info(format!("Add to {}", path.display()));
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        SERVER_NAME: cursor_server_entry()
                    }
                }))
                .unwrap_or_default()
            );
        }
        McpTarget::Codex => {
            let path = codex_config_path(McpLocation::Global);
            output::info(format!("Add to {}", path.display()));
            println!();
            println!("{}", codex_toml_snippet());
        }
    }
}

fn install_claude(location: McpLocation) -> Result<()> {
    let path = claude_config_path(location);
    let mut root = read_json_object(&path)?;
    let servers = ensure_object(&mut root, "mcpServers")?;
    servers.insert(SERVER_NAME.to_string(), claude_server_entry());
    write_json(&path, &Value::Object(root))?;
    output::success(format!("installed MCP server into {}", path.display()));
    Ok(())
}

fn uninstall_claude(location: McpLocation) -> Result<()> {
    let path = claude_config_path(location);
    if !path.exists() {
        output::info(format!("no Claude config at {}", path.display()));
        return Ok(());
    }
    let mut root = read_json_object(&path)?;
    if let Some(Value::Object(servers)) = root.get_mut("mcpServers") {
        if servers.remove(SERVER_NAME).is_some() {
            write_json(&path, &Value::Object(root))?;
            output::success(format!("removed MCP server from {}", path.display()));
            return Ok(());
        }
    }
    output::info(format!("sivtr MCP was not installed in {}", path.display()));
    Ok(())
}

fn install_cursor(location: McpLocation) -> Result<()> {
    let path = cursor_config_path(location);
    let mut root = read_json_object(&path)?;
    let servers = ensure_object(&mut root, "mcpServers")?;
    servers.insert(SERVER_NAME.to_string(), cursor_server_entry());
    write_json(&path, &Value::Object(root))?;
    output::success(format!("installed MCP server into {}", path.display()));
    Ok(())
}

fn uninstall_cursor(location: McpLocation) -> Result<()> {
    let path = cursor_config_path(location);
    if !path.exists() {
        output::info(format!("no Cursor config at {}", path.display()));
        return Ok(());
    }
    let mut root = read_json_object(&path)?;
    if let Some(Value::Object(servers)) = root.get_mut("mcpServers") {
        if servers.remove(SERVER_NAME).is_some() {
            write_json(&path, &Value::Object(root))?;
            output::success(format!("removed MCP server from {}", path.display()));
            return Ok(());
        }
    }
    output::info(format!("sivtr MCP was not installed in {}", path.display()));
    Ok(())
}

fn install_codex(location: McpLocation) -> Result<()> {
    if matches!(location, McpLocation::Local) {
        bail!("codex only supports global install currently; use --location global");
    }
    let path = codex_config_path(location);
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
    output::success(format!("installed MCP server into {}", path.display()));
    Ok(())
}

fn uninstall_codex(location: McpLocation) -> Result<()> {
    if matches!(location, McpLocation::Local) {
        bail!("codex only supports global uninstall currently; use --location global");
    }
    let path = codex_config_path(location);
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
    output::success(format!("removed MCP server from {}", path.display()));
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

fn claude_server_entry() -> Value {
    json!({
        "type": "stdio",
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn cursor_server_entry() -> Value {
    json!({
        "type": "stdio",
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn codex_toml_snippet() -> String {
    format!("[mcp_servers.{SERVER_NAME}]\ncommand = \"sivtr\"\nargs = [\"mcp\", \"serve\"]\n")
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

fn codex_config_path(_location: McpLocation) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("config.toml")
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
    fn resolves_auto_and_named_targets() {
        let named = resolve_targets("claude,cursor").expect("parse");
        assert_eq!(named, vec![McpTarget::Claude, McpTarget::Cursor]);
        assert!(resolve_targets("nope").is_err());
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
}
