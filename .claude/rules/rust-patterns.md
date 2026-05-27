# Rust Patterns — sivtr Development Rules

## Error Handling

Always `anyhow::Result` with context:

```rust
use anyhow::{Context, Result};

fn load_session(path: &Path) -> Result<Session> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("Failed to read session: {}", path.display()))?;
    serde_json::from_str(&data)
        .context("Failed to parse session JSON")
}
```

Never:
- `unwrap()` in production code
- Silent `Err(_) => {}` — always log or propagate
- `println!` in command output path — use `eprintln!` for diagnostics

## Workspace Boundary

`sivtr-core` (`crates/sivtr-core/`) must never import from `src/`. The dependency direction is:
```
src/ (CLI) → sivtr-core (library)
```

If CLI types (Clap args) need core logic, pass primitives or define shared types in core.

## String Handling

- Function params: `&str` in, `String` out
- Never `&String` as param — use `&str`
- `Path`/`PathBuf` for filesystem paths, never string concatenation

## Ownership

Borrow over clone in hot paths (search, parsing). Clone only when data must outlive the borrow.

## Regex

Compiled once at module level:
```rust
use regex::Regex;
use std::sync::LazyLock;

static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^pattern").unwrap());
```

Never `Regex::new()` inside a function body.

## Module Structure

Command modules follow:
1. Imports
2. Types (args structs)
3. Public `execute()` entry point
4. Private helpers
5. `#[cfg(test)] mod tests { ... }`

## Anti-Patterns

| Pattern | Problem | Fix |
|---------|---------|-----|
| `unwrap()` in production | Panics break user workflow | `.context()?` |
| `async fn` / tokio | Adds startup latency | Blocking I/O only |
| `Regex::new()` in fn body | Recompiles every call | `LazyLock` or `lazy_static!` |
| Silent error swallow | User gets no feedback | Log + fallback |
| `println!` in output path | Corrupts piped output | `eprintln!` |
| Core importing CLI types | Circular dependency | Move type to core |
