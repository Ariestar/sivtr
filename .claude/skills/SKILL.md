---
description: Verify sivtr environment and diagnose issues. Run when sivtr commands fail or return empty results.
allowed-tools: Bash
---

# sivtr Environment Check

Diagnose why sivtr commands may not be working as expected.

## Steps

### 1. Check binary

```bash
sivtr --version
```

If this fails, sivtr is not installed or not in PATH. Install with:
```bash
cargo install sivtr
```

### 2. Run diagnostics

```bash
sivtr doctor
```

This checks: binary version, config file, session log directory, shell hooks, provider sessions, clipboard.

### 3. Check shell hooks

If `sivtr doctor` shows hooks not installed:

```bash
sivtr init show
```

Install the hook for the current shell:
```bash
sivtr init bash    # or zsh, powershell, nushell
```

Then restart the terminal (or source the profile).

### 4. Verify provider data

If provider searches return empty:
```bash
sivtr doctor
```

The "provider sessions" section shows how many sessions each provider has. If zero:
- The provider (Codex, Claude Code, Pi, OpenCode) may not have been used in this workspace
- The session data may be in a non-standard location

### 5. Check config

```bash
sivtr config show
```

If missing:
```bash
sivtr config init
```

## Common Issues

| Symptom | Fix |
|---------|-----|
| `sivtr` not found | Install: `cargo install sivtr` |
| No terminal results | `sivtr init bash` + restart terminal |
| No AI session results | Provider may not have sessions in this workspace |
| Clipboard fails | `sivtr doctor` checks clipboard access |
| Stale results | `sivtr clear --all` resets session logs |
