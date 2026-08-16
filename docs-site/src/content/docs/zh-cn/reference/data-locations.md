---
title: 数据位置
description: sivtr 存放配置、history、session log 和 provider 数据的位置。
---

`sivtr` 是 local-first。它使用的大多数数据已经在你的机器上；它生成的数据全部存放在一个 home 目录下：

- 设置了 `SIVTR_HOME` 时用它（Grok 风格的整体搬迁），
- 否则每个平台都用 `~/.sivtr`。

```text
~/.sivtr/
  config.toml               ← 配置
  history.db                ← 捕获的终端输出（SQLite）
  sets/                     ← WorkSet 检查点（@last、@name）
  workspaces/<key>/terminals/  ← shell session log（session_<pid>.jsonl）
  cache/                    ← agent session 解析缓存
  identity.key              ← 远程记忆的设备身份
  remote-state.db           ← peers、shares、grants、invites、mounts
  daemon.json / daemon.lock / daemon.log
```

请通过 CLI 命令访问，而不是直接编辑这些文件。

## 从旧版本迁移

sivtr ≤ 0.4 把数据分散在平台的 config、state 和 data 目录。`sivtr doctor` 会报告仍然留在旧位置的数据，`sivtr doctor --fix` 会把它们搬进统一 home。迁移按目录合并、绝不覆盖已存在的目标；冲突项会被报告并留在原地。

## 配置文件

```text
<SIVTR_HOME 或 ~/.sivtr>/config.toml
```

## Shell session log

Shell 集成会写入按进程区分的结构化 session log，位于 `<home>/workspaces/<key>/terminals/session_<pid>.jsonl`。

这些 log 支撑：

- `sivtr import`；
- `sivtr copy` 命令块工作流；
- `sivtr diff`；
- browser 中的命令块导航。

## History 数据库

当 `[history].auto_save = true` 时，捕获的终端输出会保存到本地 SQLite history 数据库：

```text
<home>/history.db
```

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
| Hermes | 主路径：`$HERMES_HOME/state.db`（Windows 默认 `%LOCALAPPDATA%\hermes`，其他平台 `~/.hermes`）。Residual：同目录下 `sessions/*.jsonl`。 |
| OpenCode | OpenCode 本地数据库 |
| Cursor | `~/.cursor/projects/**/agent-transcripts/**/*.jsonl`（可用 `CURSOR_HOME` 覆盖） |
| OpenClaw | `~/.openclaw/agents/<id>/agent/openclaw-agent.sqlite`（legacy JSONL 在 `sessions/`） |
| Grok | `~/.grok/sessions/**`（`summary.json` + `chat_history.jsonl`；可用 `GROK_HOME` 覆盖） |
| Pi | Pi agent session JSONL 文件 |

各 provider 格式不同；`sivtr` 会把它们归一化为 session 和 dialogue unit，用于 copy、picker、search 和 show 工作流。

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

Windows hotkey 状态存放在统一 home 下，由以下命令管理：

```bash
sivtr hotkey status
sivtr hotkey stop
```

## Remote daemon 状态

跨设备远程记忆使用设备级 daemon。状态存放在统一 home 下：

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

远程访问是 opt-in。只有 `sivtr share`（或 `share add`）之后才会分享；邀请用 `share invite` 签发。remote 是用 `sivtr remote add` 登记的 workspace 本地名。功能指南见 [远程访问](/zh-cn/usage/remote-access/)。
