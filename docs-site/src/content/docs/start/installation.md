---
title: Installation
description: Install sivtr with Cargo and set up shell integration.
---

`sivtr` is published as a Cargo package and developed at [github.com/Ariestar/sivtr](https://github.com/Ariestar/sivtr).

## Requirements

- Rust and Cargo
- A supported terminal
- Clipboard support for your platform

Optional:

- `nvim`, `vim`, or `vi` for the Vim picker view used by some copy workflows
- PowerShell, Bash, Zsh, or Nushell shell profile access for session logging

## Install with Cargo

Install the latest published release from crates.io:

```bash
cargo install sivtr
```

Verify the binary:

```bash
sivtr --version
sivtr --help
```

## Install from source

Clone the repository:

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
```

From the repository root:

```bash
cargo install --path .
```

## Update

Update the published package:

```bash
cargo install sivtr --force
```

Or reinstall from a local checkout after pulling changes:

```bash
git pull
cargo install --path . --force
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
