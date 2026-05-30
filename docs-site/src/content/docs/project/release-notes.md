---
title: Release Notes
description: User-facing release notes for sivtr.
---

`sivtr` is in early `0.1.x` development. The CLI and configuration format may still change during this series. This page summarizes user-facing changes; the repository `CHANGELOG.md` remains the detailed changelog source.

## Unreleased

### Added

- Added `sivtr filter` as a first-class WorkSet filtering command for saved variables, source selectors, and stdin `@` pipelines.
- Added `sivtr var` for named WorkSet memory: `set`, `list`, `rm`, `merge`, `drop`, and `cleanup`.
- Added `sivtr nav` for deterministic anchor motion with `<`, `>N`, `+N`, `-N`, `[A..B]`, and `~`. `nav` does not implicitly expand child anchors.
- Added global `--color auto|always|never` for status and diagnostic output.

### Changed

- Standardized CLI status and diagnostic messages, keeping them on stderr so stdout remains safe for data pipelines.
- `search` and `work parts` now share the same WorkSet filtering implementation.

### Fixed

- Fixed Nushell shell integration on Windows so captured command output is no longer empty when the prompt is visible but Nushell does not expose prompt-closure environment mutations to later hooks.

## 0.1.3 - 2026-05-24/25

### Added

- Added git-root workspace scoping for terminal records so terminals opened below the same repository resolve to the same workspace.
- Added `sivtr init all` / `sivtr init -all` for installing all supported shell hooks at once.
- Added target-first search syntax: `sivtr search terminal|agent|codex|claude|opencode|pi ...`.
- Added target path narrowing down to session, record/turn, and line refs, for example `terminal/session_13104/3/12`.
- Added search filters for field (`--in`), status, exit code, min/max duration, cwd, time ranges, latest records, limit, current-session exclusion, and sorting.
- Added natural local time aliases for search ranges, including `today`, `yesterday`, `tomorrow`, `this morning`, `this afternoon`, `this evening`, `tonight`, and `now`.
- Added `--format timeline|compact|md|json` for search output. `json` remains the machine-readable default.
- Added stable search JSON snippets without the redundant `line` field in matches.
- Added OpenCode and Pi agent search/copy coverage alongside Codex and Claude Code.
- Added `WorkTime` with `started_at`, `ended_at`, and `duration_ms`, deriving the third component when two are available.
- Added `sivtr version --verbose` to print binary path, profile, git/build metadata, repo root, and local debug-binary diagnostics.

### Changed

- Search now treats target selection and filtering as separate concerns. Old `--scope`, `--provider`, `--recent`, and `--json` search flags were removed in favor of target-first syntax, `--latest`, and `--format json`.
- Search/show timestamps are normalized to local RFC3339 with offset.
- Agent record titles skip `[skill:...]` marker lines and prefer the real user request.
- Skill injection content is compacted in records so prompts do not dominate titles and search snippets.
- WorkRecord was simplified around a stable top-level `work_ref`, less duplicated source/id data, and structured text/payload fields.
- Search results are grouped by record/dialogue with snippets to reduce duplicate line noise.

### Fixed

- Fixed terminal search returning empty results when shell history timestamps used local PowerShell-style strings such as `Mon May 25 00:35:02 2026`.
- Fixed interrupted agent turns so they stay searchable.
- Fixed clippy warnings after the record/time/search refactors.

## 0.1.3 - 2026-05-20

### Added

- Added the workspace picker experience for browsing agent sessions with richer content rendering, search navigation, scrolling, and line-numbered content views.
- Added workspace copy shortcuts for agent sessions: `i` copies user input, `o` copies assistant output, and `y` copies the whole dialogue block without role headings.
- Added project roadmap pages to the documentation site.

### Fixed

- Hardened VS Code picker command quoting across PowerShell, cmd.exe, fish, and POSIX shells.
- Ignored Claude `ai-title` metadata events instead of failing session parsing.
- Fixed CI clippy warnings.

## 0.1.2 - 2026-05-02

### Fixed

- Treat cancelling interactive pickers as a normal exit.

## 0.1.1 - 2026-05-01

### Fixed

- Fixed Codex copy picker TUI selection logic.
- Fixed terminal exit handling that could leave the terminal stuck.

## 0.1.0 - 2026-04-28

### Added

- Added `sivtr`, an early agent memory workspace for capturing command output and agent coding sessions.
- Added pipe mode with `command | sivtr`.
- Added run mode with `sivtr run <command>`.
- Added Vim-style navigation, modal interaction, visual selection, search, and clipboard copy.
- Added local SQLite history with full-text search.
- Added Codex session capture helpers with `sivtr copy codex` for reusing assistant replies, user prompts, and tool output.
- Added command-block copy, diff, and picker workflows.
- Added TOML configuration support.
- Added Windows global hotkey support for the Codex picker workflow.

## Current documented surface

The current docs cover:

- terminal pipe and run capture;
- shell session logging;
- TUI browsing and selection;
- command-block copy and diff;
- agent session copy and picker workflows for Codex, Claude Code, OpenCode, and Pi;
- workspace search and show refs;
- SQLite terminal history;
- TOML configuration;
- Windows hotkey, VS Code, tmux, Linux shortcut, and macOS launcher flows.
