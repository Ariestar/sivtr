---
title: 数据位置
description: sivtr 存放配置、history、session log 和 provider 数据的位置。
---

`sivtr` 是 local-first。它使用的大多数数据已经在你的机器上；它生成的数据默认写在平台配置或状态目录下，除非你显式导出到其他位置。

## 配置文件

| 平台 | 路径 |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

## Shell session log

Shell 集成会写入按进程区分的结构化 session log。

| Shell/平台 | 常见路径 |
| --- | --- |
| Windows PowerShell / PowerShell 7 | `%APPDATA%\sivtr\session_<pid>.log` |
| Bash / Zsh | `$XDG_STATE_HOME/sivtr/session_<pid>.log` 或 `~/.local/state/sivtr/session_<pid>.log` |
| Nushell | Nushell config/state 区域中的 `sivtr` session 文件 |

这些 log 支撑：

- `sivtr import`；
- `sivtr copy` 命令块工作流；
- `sivtr diff`；
- browser 中的命令块导航。

## History 数据库

当 `[history].auto_save = true` 时，捕获的终端输出会保存到本地 SQLite history 数据库。

请通过 CLI 命令访问，而不是直接编辑数据库：

```bash
sivtr history list
sivtr history search "panic"
sivtr history show 42
```

保留策略由以下配置控制：

```toml
[history]
max_entries = 0
```

`0` 表示不限制数量。

## Agent provider 数据

`sivtr` 读取 provider 自己的本地数据，不上传 transcript。

| Provider | 数据来源 |
| --- | --- |
| Codex | `~/.codex/sessions` rollout JSONL 文件 |
| Claude Code | 当前 transcript/session 环境变量和本地 Claude transcripts |
| Hermes | `$HERMES_HOME/sessions`；Windows 默认 `%LOCALAPPDATA%\hermes\sessions`，其他平台默认 `~/.hermes/sessions` |
| OpenCode | OpenCode 本地数据库 |
| Pi | Pi agent session JSONL 文件 |

各 provider 格式不同；`sivtr` 会把它们归一化为 session 和 dialogue unit，用于 copy、picker、search 和 show 工作流。

## Claude 导出导入

`sivtr import claude-export` 默认把确定性批次写入：

```text
~/.claude/projects/<编码后的 cwd>/sivtr-imports/<batch-id>/
```

每个批次包含 Claude 可读取的 JSONL session、与源文件逐字节一致的快照，以及记录哈希、消息和分支映射的 manifest。已有批次永不覆盖；可以用 `--dest` 将批次根目录放到隔离位置进行验证或归档。

## Codex 导出 mirror

`codex export` 会把本地 Codex session 文件复制到你选择的目标目录：

```bash
sivtr codex export --dest /srv/sivtr/root-codex
```

目标目录会包含一个 `sessions/` 树。另一个账号可通过配置读取：

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

共享 mirror 尽量使用只读权限。

## 生成的启动器

Linux shortcut generation 会写入：

- `~/.local/bin/sivtr-pick-codex`；
- `~/.local/share/applications/sivtr-pick-codex.desktop`。

macOS shortcut generation 会写入：

- `~/.local/bin/sivtr-pick-codex`；
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`。

Windows hotkey 状态存放在 `sivtr` 的平台 config/state 区域下，由以下命令管理：

```bash
sivtr hotkey status
sivtr hotkey stop
```

## Remote daemon 状态

跨设备远程记忆使用设备级 daemon。可用 `SIVTR_DATA_DIR` 覆盖根目录；否则是平台 config 目录下的 `sivtr`（与 `data_dir()` 相同）。

| 文件 | 用途 |
| --- | --- |
| `identity.key` | iroh 使用的稳定设备身份 |
| `remote-state.db` | SQLite：peers、shares、grants、invites、mounts、audit |
| `daemon.json` | 运行中 daemon 控制信息（port、token、node id） |
| `daemon.lock` | 单实例锁 |
| `daemon.log` | daemon 日志（`sivtr serve logs`） |

```bash
sivtr serve status
sivtr serve logs
sivtr share list
sivtr remote list
sivtr peer list
sivtr ws list
```

远程访问是 opt-in。只有 `sivtr share`（或 `share add`）之后才会分享。mount 是用 `sivtr remote add` 登记的 workspace 本地别名。功能指南见 [远程访问](/zh-cn/usage/remote-access/)。
