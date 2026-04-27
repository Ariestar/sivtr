---
title: Installation
description: Install sivtr from source and set up shell integration.
---

`sivtr` is a Rust workspace with a CLI/TUI binary and a core library crate. The current installation path is from source.

## Requirements

- Rust toolchain compatible with the repository `rust-toolchain.toml`
- Cargo
- A supported terminal
- Clipboard support for your platform

Optional:

- `nvim`, `vim`, or `vi` for the Vim picker view used by some copy workflows
- PowerShell, Bash, Zsh, or Nushell shell profile access for session logging

## Install from the repository

From the repository root:

```bash
cargo install --path .
```

Verify the binary:

```bash
sivtr --version
sivtr --help
```

## Update after pulling changes

Reinstall from the repository root:

```bash
cargo install --path .
```

Cargo will replace the previously installed binary.

## Shell integration

Shell integration records recent command blocks so `sivtr copy`, `sivtr import`, and command-block navigation have structured data to work with.

Install the hook for your shell:

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

Restart the terminal after installation.

The hook writes a per-process session log:

- Windows PowerShell and PowerShell 7 use `%APPDATA%\sivtr\session_<pid>.log`.
- Bash and Zsh use `$XDG_STATE_HOME/sivtr/session_<pid>.log` or `~/.local/state/sivtr/session_<pid>.log`.
- Nushell uses its config directory with a `sivtr` session file.

## Configuration file

Create the default config file:

```bash
sivtr config init
```

Show the path and current content:

```bash
sivtr config show
```

Edit it with your configured editor:

```bash
sivtr config edit
```

See [Config File](/reference/config-file/) for all supported settings.
