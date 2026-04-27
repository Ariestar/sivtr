---
title: Architecture
description: How the sivtr workspace is split between CLI, TUI, and core modules.
---

`sivtr` is a Cargo workspace with two main layers:

- `sivtr`, the binary crate in `src/`;
- `sivtr-core`, the library crate in `crates/sivtr-core/`.

The binary owns user interaction: CLI parsing, command dispatch, TUI state, and platform-specific hotkey behavior. The core crate owns reusable logic: capture, parsing, buffers, selection, search, history, export, config, and Codex session parsing.

## Workspace layout

```text
sivtr/
|- Cargo.toml
|- src/
|  |- cli.rs
|  |- main.rs
|  |- app.rs
|  |- commands/
|  `- tui/
`- crates/
   `- sivtr-core/
      `- src/
         |- buffer/
         |- capture/
         |- config/
         |- export/
         |- history/
         |- parse/
         |- search/
         |- selection/
         |- session/
         `- codex.rs
```

## Binary crate

The binary crate contains:

| Area | Responsibility |
| --- | --- |
| `cli.rs` | clap command definitions and help text |
| `commands/` | command handlers for run, pipe, copy, history, config, hotkey, diff, and import |
| `app.rs` | TUI state machine |
| `tui/` | terminal setup, event handling, and rendering |
| `command_blocks.rs` | parsed command-block spans for session browsing and copying |

This layer can depend on terminal UI libraries, platform APIs, and process spawning behavior.

## Core crate

The core crate contains reusable domain logic:

| Module | Responsibility |
| --- | --- |
| `capture` | stdin, subprocess, and scrollback/session capture helpers |
| `parse` | ANSI stripping, Unicode display width, and line parsing |
| `buffer` | line, cursor, and viewport models |
| `selection` | visual, line, and block selection extraction |
| `search` | matching and navigation state |
| `history` | SQLite storage, schema, and search |
| `export` | clipboard, file, and editor export helpers |
| `config` | TOML config model, defaults, and path resolution |
| `session` | structured session entries and rendering |
| `codex` | Codex session discovery, parsing, and formatting |

This split keeps computation and data handling in Rust modules that can be tested independently from the TUI.

## Capture flow

Pipe mode:

```text
stdin -> capture::pipe -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Run mode:

```text
subprocess -> combined output -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Session import:

```text
session log -> render entries -> parse::parse_lines -> Buffer -> command block spans -> TUI/editor
```

Copy mode:

```text
session log -> SessionEntry list -> command blocks -> selector -> filters -> clipboard
```

Codex copy:

```text
~/.codex/sessions -> current cwd match -> parsed blocks -> selector -> filters -> clipboard
```

## Design boundary

The frontend layer is presentation and interaction. The Rust core performs the computation: parsing, capture, selection extraction, search, storage, and formatting. This keeps UI changes from leaking into the data model and makes command behavior easier to test.
