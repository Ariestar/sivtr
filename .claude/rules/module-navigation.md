# Module Navigation — sivtr Codebase

## Priority Order

1. **Grep** (exact symbol) → known function/type names
2. **Glob** (file discovery) → finding modules by name
3. **Read** (full file) → only after locating the right file
4. **Explore agent** → last resort for >3 queries

## Module Map

```
src/
├── main.rs                    ← Command routing (start here for any command)
├── cli.rs                     ← All Clap definitions (~2000 lines)
├── app.rs                     ← Application state
├── command_blocks.rs          ← Command block parsing and selection
├── commands/
│   ├── init.rs                ← Shell hook injection + show/uninstall
│   ├── copy.rs                ← Copy command + workspace picker
│   ├── copy/workspace_picker.rs ← Interactive picker TUI
│   ├── search.rs              ← Search command with pipeline
│   ├── show.rs                ← Show ref content
│   ├── diff.rs                ← Diff two blocks
│   ├── work.rs                ← Workspace traversal (sessions/records/parts)
│   ├── work_json.rs           ← JSON metadata helpers for work command
│   ├── config.rs              ← Config init/show/edit
│   ├── doctor.rs              ← Environment diagnostics
│   ├── hotkey.rs              ← Global hotkey (Windows)
│   ├── flush.rs               ← Internal: shell hook callback
│   ├── browse.rs              ← TUI browse entry
│   ├── records.rs             ← Record display
│   ├── import.rs              ← Import session log
│   ├── history.rs             ← History management
│   ├── version.rs             ← Version diagnostics
│   ├── time_filter.rs         ← Time filter parsing
│   ├── run.rs / pipe.rs       ← Run and pipe commands
│   └── clear.rs               ← Clear session logs
├── tui/                       ← Terminal UI framework
│   ├── mod.rs / terminal.rs   ← TUI core
│   ├── views/                 ← Browse, search, status views
│   ├── workspace.rs           ← Workspace data model for TUI
│   └── workspace_search.rs    ← Workspace search in TUI
│
crates/sivtr-core/src/
├── lib.rs                     ← Core library root
├── ai.rs                      ← AgentProvider, session parsing, block selection
├── claude.rs                  ← Claude Code JSONL parser
├── codex.rs                   ← Codex rollout JSONL parser
├── opencode.rs                ← OpenCode session parser
├── pi.rs                      ← Pi session parser
├── record/
│   ├── model.rs               ← WorkRecord, WorkPart, WorkTime (data model center)
│   ├── refs.rs                ← WorkRef parsing and formatting
│   ├── index.rs               ← Record indexing
│   └── mod.rs                 ← Re-exports
├── search/
│   ├── mod.rs                 ← Search types and orchestration
│   ├── matcher.rs             ← Content matching logic
│   └── navigator.rs           ← Workspace navigation for search
├── workspace.rs               ← Workspace resolution
├── config/                    ← SivtrConfig, key bindings
├── session.rs                 ← Session log reading
├── session/                   ← Session entry types and capture
├── history/                   ← SQLite command history store
├── export/                    ← Clipboard, editor, file export
├── capture/                   ← Terminal capture (scrollback, pipe, subprocess)
├── buffer/                    ← Text buffer with cursor and viewport
├── selection/                 ← Text selection and extraction
├── parse/                     ← ANSI stripping, unicode width
└── time.rs                    ← Timestamp parsing and normalization
```

## Common Search Patterns

### "Where is command X handled?"
```
Grep pattern="Search\b\|Copy\b\|Init\b" path="src/main.rs"
```

### "Where is function X defined?"
```
Grep pattern="fn execute\b|fn filter_" type="rust"
```

### "All command modules"
```
Glob pattern="src/commands/*.rs"
```

### "Record model tests"
```
Grep pattern="#\[cfg(test)\]" path="crates/sivtr-core/src/record/model.rs"
```

### "Provider session discovery"
```
Grep pattern="find_sessions\|discover_sessions" type="rust"
```

## Anti-Patterns

- Don't read all command files to find one function — Grep first
- Don't use Bash `find` or `grep` — use dedicated tools
- Don't read `cli.rs` end-to-end — Grep for the specific arg definition
