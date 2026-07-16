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
├── app.rs                     ← Application state
├── command_blocks.rs          ← Command block parsing and selection
├── commands/
│   ├── capture/               ← Shell capture + copy/diff/import/init/flush/browse
│   │   ├── copy.rs            ← Copy command + workspace picker
│   │   ├── copy/workspace_picker.rs
│   │   ├── copy/vim.rs
│   │   ├── init.rs            ← Shell hook injection + show/uninstall
│   │   ├── run.rs / pipe.rs   ← Run and pipe commands
│   │   ├── flush.rs           ← Internal: shell hook callback
│   │   ├── import.rs / clear.rs / diff.rs / browse.rs
│   │   └── command_block_selector.rs
│   ├── memory/                ← WorkSet / search / show surface
│   │   ├── search.rs / filter.rs / var.rs / nav.rs / zoom.rs
│   │   ├── show.rs / work.rs / work_json.rs / records.rs
│   │   ├── time_filter.rs
│   │   └── workset/           ← WorkSet source resolution + store
│   ├── remote/                ← Device daemon CLI surface
│   │   ├── serve.rs           ← start/stop/restart/status/logs/foreground
│   │   ├── share.rs           ← interactive share + add/list/invite/grants/revoke
│   │   ├── mounts.rs          ← remote add/list/remove/rename/test
│   │   ├── peer.rs            ← peer list/forget
│   │   └── workspace.rs       ← ws list + local origin name resolution
│   └── system/                ← config, doctor, history, hotkey, codex, migrate, version
├── remote/                    ← Daemon runtime (not CLI handlers)
│   ├── daemon.rs              ← iroh remote + localhost control plane
│   ├── state.rs               ← SQLite peers/shares/grants/invites/mounts
│   ├── identity.rs            ← stable device key
│   ├── protocol.rs            ← LocalRequest/RemoteRequest types + InviteTicket
│   └── ipc.rs                 ← CLI → daemon control IPC
└── tui/                       ← Terminal UI framework
    ├── mod.rs / terminal.rs   ← TUI core
    ├── views/                 ← Browse, search, status views
    ├── workspace.rs           ← Workspace data model for TUI
    └── workspace_search.rs    ← Workspace search in TUI

crates/sivtr-core/src/
├── lib.rs                     ← Core library root
├── ai.rs                      ← AgentProvider, session parsing, block selection
├── claude.rs                  ← Claude Code JSONL parser
├── codex.rs                   ← Codex rollout JSONL parser
├── hermes.rs                  ← Hermes session parser
├── opencode.rs                ← OpenCode session parser
├── pi.rs                      ← Pi session parser
├── record/
│   ├── model.rs               ← WorkRecord, WorkPart, WorkTime (data model center)
│   ├── refs.rs                ← WorkRef = WorkScope + WorkPath + WorkAt ([scope:]path[/at])
│   ├── index.rs               ← Record indexing
│   └── mod.rs                 ← Re-exports
├── query/                     ← load_workspace_records / load_workspace_source
├── search/
│   ├── mod.rs                 ← Search types and orchestration
│   ├── matcher.rs             ← Content matching logic
│   └── navigator.rs           ← Workspace navigation for search
├── workspace.rs               ← Workspace resolution + data_dir()
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
