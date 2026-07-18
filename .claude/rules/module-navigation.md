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
├── cli/
│   ├── mod.rs                 ← Top-level Clap definitions
│   └── remote.rs              ← Serve/Share/Peer/Remote/Workspace Clap types
├── commands/
│   ├── terminal/              ← Write terminal memory
│   │   ├── init.rs            ← Shell hook injection + show/uninstall
│   │   ├── flush.rs           ← Hook callback: write session log
│   │   ├── clear.rs           ← Clear session logs
│   │   ├── run.rs / pipe.rs / import.rs  ← One-shot ingest → history + editor
│   │   └── history.rs         ← Optional history auto-save helper
│   ├── copy/                  ← Export to clipboard (plan/load/project/export)
│   ├── select.rs              ← Relative dialogue select (1 / A..B)
│   ├── diff.rs                ← Compare two terminal dialogues
│   ├── browse/                ← Product TUI (bare `sivtr` / hotkey / pick)
│   │   ├── mod.rs / load.rs / picker.rs / selection.rs / content.rs
│   │   └── help.rs / nav / vim / visual / text
│   ├── memory/                ← WorkSet / search / show surface
│   │   ├── search.rs / filter.rs / var.rs / nav.rs / zoom.rs
│   │   ├── show.rs / work.rs / work_json.rs / records.rs
│   │   ├── time_filter.rs
│   │   └── workset/           ← WorkSet source resolution + store
│   ├── remote/                ← Device daemon CLI surface
│   │   ├── serve.rs / share.rs / mounts.rs / peer.rs / workspace.rs
│   └── system/                ← config, doctor, history, hotkey, codex, …
├── remote/                    ← Daemon runtime (not CLI handlers)
│   ├── daemon.rs / state.rs / identity.rs / protocol.rs / ipc.rs
└── tui/                       ← Workspace browser rendering (not product entry)
    ├── terminal.rs / theme.rs / pane.rs
    ├── content_view.rs / content_markdown.rs
    ├── workspace.rs / workspace_search.rs

crates/sivtr-core/src/
├── lib.rs                     ← Core library root
├── agents/                    ← AgentProvider registry + per-provider parsers
├── record/                    ← WorkRecord, WorkRef, index
├── query/                     ← load_workspace_records / load_workspace_source (terminal+agent)
├── search/                    ← Search matcher / navigator
├── workspace.rs               ← Workspace resolution + data_dir()
├── config/                    ← SivtrConfig
├── session.rs / session/      ← Session log types
├── history/                   ← SQLite history store
├── export/                    ← Clipboard, editor, file export
├── capture/                   ← Low-level terminal capture (scrollback, pipe, subprocess)
├── buffer/ / selection/ / parse/  ← Text primitives (shared)
└── time.rs
```

**Read vs write:**
- Write terminal memory → `commands/terminal`
- Read any source (terminal + agents) → `memory/workset` + `sivtr-core::query`
- Interactive pick → `commands/browse`
- Clipboard export → `commands/copy`

## Common Search Patterns

### "Where is command X handled?"
```
Grep pattern="Search\b|Copy\b|Init\b|Share\b|Serve\b" path="src/main.rs"
```

### "Where is function X defined?"
```
Grep pattern="fn execute\b|fn filter_" type="rust"
```

### "All command modules"
```
Glob pattern="src/commands/**/*.rs"
```

### "Record model tests"
```
Grep pattern="#\[cfg(test)\]" path="crates/sivtr-core/src/record/model.rs"
```

### "Provider session discovery"
```
Grep pattern="find_sessions|discover_sessions" type="rust"
```

### "Remote daemon / share / mount"
```
Grep pattern="ShareAdd|RemoteAdd|InviteTicket|StateStore" type="rust"
```

## Anti-Patterns

- Don't read all command files to find one function — Grep first
- Don't use Bash `find` or `grep` — use dedicated tools
- Don't read `cli/mod.rs` end-to-end — Grep for the specific arg definition
- Don't assume remote lives under `src/commands/` only — daemon runtime is `src/remote/`
