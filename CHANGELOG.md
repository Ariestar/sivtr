# Changelog

All notable user-facing changes to this project are documented here.

## [0.1.0] - 2026-04-28

### Added

- Added `sivtr`, a terminal output workspace for capturing command output and AI coding sessions.
- Added pipe mode with `command | sivtr`.
- Added run mode with `sivtr run <command>`.
- Added Vim-style navigation, modal interaction, visual selection, search, and clipboard copy.
- Added local SQLite history with full-text search.
- Added Codex session capture helpers with `sivtr copy codex` for reusing assistant replies, user prompts, and tool output.
- Added command-block copy, diff, and picker workflows.
- Added TOML configuration support.
- Added Windows global hotkey support for the Codex picker workflow.

### Notes

- This is the first public release. The CLI and configuration format may still change during the `0.1.x` series.
