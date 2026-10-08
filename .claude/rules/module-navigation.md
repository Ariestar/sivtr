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
├── output.rs                  ← Status output + the stderr diagnostics listener
├── origins.rs                 ← Origin registry (local workspaces, remote mounts)
├── cli/
│   ├── mod.rs                 ← Top-level Clap definitions
│   ├── remote.rs              ← Serve/Share/Peer/Remote/Workspace Clap types
│   └── pty.rs / mcp.rs / publish.rs
├── commands/
│   ├── terminal/              ← Write terminal memory
│   │   ├── init.rs            ← Shell hook injection + show/uninstall
│   │   ├── pty_proxy.rs       ← 采集代理 CLI：run / report / enable / disable
│   │   ├── clear.rs           ← Clear session logs
│   │   └── run.rs / pipe.rs   ← One-shot ingest
│   ├── memory/                ← WorkSet / search / show / copy / diff
│   │   ├── copy/              ← Export to clipboard (plan/load/project/export)
│   │   ├── diff.rs            ← Terminal-only dialogue compare (workset load)
│   │   ├── search.rs / filter.rs / var.rs / nav.rs / zoom.rs
│   │   ├── show.rs / work.rs / work_json.rs / records.rs
│   │   ├── semantic.rs / eval.rs / time_filter.rs
│   │   └── workset/           ← WorkSet source resolution + store
│   ├── select.rs              ← Relative dialogue select (1 / A..B)
│   ├── browse/                ← Product TUI (bare `sivtr` / hotkey / pick)
│   │   ├── mod.rs / load.rs / picker.rs / selection.rs / content.rs / panes.rs
│   │   └── help.rs / nav.rs / vim.rs / visual.rs / text.rs / publish_overlay.rs
│   ├── publish/               ← Privacy-projected shared publications
│   ├── remote/                ← Device daemon CLI surface
│   │   └── serve.rs / share.rs / mounts.rs / peer.rs / group.rs / origin.rs / workspace.rs
│   └── system/                ← config, doctor, export, hotkey, import, mcp, quality,
│                                session, setup, skill, stats, sync, update, usage, version
├── mcp/                       ← MCP server (server.rs = tool handlers, types.rs)
├── pty/                       ← Capture proxy: owns the pty, records OSC 133 blocks
├── remote/                    ← Daemon runtime (not CLI handlers)
│   ├── daemon.rs / identity.rs / protocol.rs / ipc.rs / net.rs
│   └── fanout.rs / groups.rs / redact.rs / context.rs / state/
└── tui/                       ← Terminal UI framework (not product entry)
    ├── terminal.rs / theme.rs / pane.rs / panic.rs / search.rs
    └── content/ / workspace/

crates/sivtr-core/src/
├── lib.rs                     ← Core library root
├── agents/                    ← AgentProvider registry + per-provider parsers
├── archive/                   ← Unified SQLite archive: schema, store, sync, stats
├── record/                    ← WorkRecord, WorkRef, index
├── query/                     ← load_workspace_records / load_workspace_source (terminal+agent)
├── search/                    ← Filter/Searcher pipeline, BM25 ranking + index cache
├── usage/                     ← Token extraction, pricing
├── workset.rs                 ← WorkSet / selection model
├── workspace.rs               ← Workspace resolution + home_dir()
├── cache.rs / diagnostics.rs  ← On-disk cache helpers; process-wide warning sink
├── origin.rs / session_source.rs / publication.rs / privacy.rs
├── config/                    ← SivtrConfig
├── session.rs / session/      ← Session log types
├── export/                    ← Clipboard, editor, file export
├── capture/                   ← Low-level one-shot capture (pipe, subprocess)
└── time.rs
```

**Read vs write:**
- Write terminal memory → `commands/terminal`
- Read any source (terminal + agents) → `memory/workset` + `sivtr-core::query`
- Interactive pick → `commands/browse`
- Clipboard export → `commands/memory/copy`
- Terminal dialogue diff → `commands/memory/diff` (terminal-only)

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
