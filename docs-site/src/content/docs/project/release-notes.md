---
title: Release Notes
description: A lightweight changelog template for sivtr releases.
---

Use this page as the public release-notes home. Keep detailed version entries in `CHANGELOG.md` when the project starts tagging releases.

## Current status

`sivtr` is in early `0.1.x` development. The documented V1 surface includes:

- pipe and run capture modes;
- Vim-style TUI browsing and selection;
- structured shell session logging;
- command-block copy, diff, and picker workflows;
- Codex session copy workflows;
- SQLite history search;
- TOML configuration;
- Windows global Codex picker hotkey.

## Recommended changelog format

```markdown
## [Unreleased]

### Added

### Changed

### Fixed

### Removed
```

For each released version, add date and user-facing changes:

```markdown
## [0.1.1] - 2026-04-28

### Added

- Added `sivtr copy codex out --pick`.

### Fixed

- Fixed prompt rendering for multiline prompts.
```

Write release notes for users. Avoid dumping raw commit messages.
