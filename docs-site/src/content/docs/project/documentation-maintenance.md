---
title: Documentation Maintenance
description: How to keep the docs site accurate as the Rust tool changes.
---

This site should stay close to the code. The riskiest pages are command reference, keybindings, and config reference because they mirror Rust definitions.

## Source of truth

| Documentation page | Code source |
| --- | --- |
| CLI Reference | `src/cli.rs` |
| Keybindings | `src/tui/event.rs`, `src/app.rs`, `src/commands/copy.rs` |
| Config File | `crates/sivtr-core/src/config/mod.rs` |
| Session Model | `crates/sivtr-core/src/session/entry.rs` |
| Architecture | workspace layout and `crates/sivtr-core/src/lib.rs` |

## Update checklist

When changing the CLI:

1. Update clap help in `src/cli.rs`.
2. Update [CLI Reference](/reference/cli/).
3. Add or update examples in usage pages.
4. Run the docs build.

When changing TUI keys:

1. Update [Keybindings](/reference/keybindings/).
2. Update task pages that mention the changed keys.
3. Check the quickstart still works.

When changing config:

1. Update [Config File](/reference/config-file/).
2. Update [Configuration](/usage/configuration/).
3. Make sure `sivtr config show` output still matches the docs.

## Build locally

From `docs-site/`:

```bash
npm install
npm run build
npm run dev
```

## Publish

The generated site is static. Any static host works:

- GitHub Pages;
- Cloudflare Pages;
- Vercel;
- Netlify;
- a plain static file server.

The build output is:

```text
docs-site/dist/
```
