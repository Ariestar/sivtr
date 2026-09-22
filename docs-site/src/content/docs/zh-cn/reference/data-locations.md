---
title: 数据位置
description: sivtr 存放配置、统一 archive、session log 和 provider 数据的位置。
---

`sivtr` 是 local-first。它使用的大多数数据已经在你的机器上。生成的数据在一个 home 下：设置了 `SIVTR_HOME` 时用它，否则每个平台都是 `~/.sivtr`。

```text
<SIVTR_HOME 或 ~/.sivtr>/
  config.toml
  identity.key
  remote-state.db
  publication-state.db
  sets/                      # 命名 WorkSet（@last、@name）
  workspaces/                # 终端 session log
  cache/
    archive.db               # 搜索索引；也保存一次性 pipe/run capture
    bm25-*.bin               # 可删
  daemon.json / daemon.lock / daemon.log
```

`sivtr doctor --fix` 会把旧平台 config/state 目录里残留的文件迁进这个 home。不要删 `workspaces/`、`sets/` 或 `identity.key`。删 `bm25-*.bin` 只会触发重建。`archive.db` 里还有不在 `workspaces/` 的一次性 `pipe`/`run` capture。

## 配置文件

| 平台 | 路径 |
| --- | --- |
| 所有平台 | `~/.sivtr/config.toml`（可用 `SIVTR_HOME` 覆盖） |

## Shell session log

通过 `sivtr init` 或 `sivtr setup` 安装 shell 集成并重启后，shell 默认运行在 `sivtr pty-proxy run` 之内，每条执行完的命令会向按终端区分的日志追加一条结构化记录：

| 常见路径 |
| --- |
| `<home>/workspaces/<workspace-key>/terminals/<terminal_id>.jsonl` |

`<workspace-key>` 标识 git 仓库 —— 同一仓库的所有 worktree 共用同一个 key；`<terminal_id>` 标识 shell 会话，因此不同终端不会互相混淆。不在 git 仓库内时不会记录任何内容。

这些 log 支撑：

- `sivtr` workspace 浏览器；
- `sivtr copy` 命令块工作流；
- `sivtr diff`；
- browser 中的命令块导航。

`sivtr clear` 删除当前终端的日志；`sivtr clear --all` 删除全部 workspace 会话树。

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

## 统一 archive

search、show、copy、picker、TUI 和 MCP 查询都从统一的本地 archive 读取，而不是每次解析原生 session 文件。

| 路径 |
| --- |
| `<home>/cache/archive.db` |

它是一个 SQLite 数据库（WAL 模式），由 sync 引擎写入：`sivtr sync` 显式执行一次同步，查询在 archive 比 `[sync].max_age_secs` 更旧时也会自动执行新鲜度同步，`pipe`/`run` 也会直接写入一次性 terminal capture。原生 Agent session 文件和 shell session log 仍是 source of truth，sync 引擎读取它们；当 session 在 archive 中缺失或过期时，按 session 寻址的加载会通过解析原生文件自愈。`cache/` 下的 BM25 文件可删后重建。若还需要一次性 `pipe`/`run` capture，不要删 `archive.db`。
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

跨设备远程记忆使用设备级 daemon。文件在统一 home 下（`SIVTR_HOME` / `~/.sivtr`）。

| 文件 | 用途 |
| --- | --- |
| `identity.key` | iroh 使用的稳定设备身份 |
| `remote-state.db` | SQLite：peers、shares、grants、invites、mounts、audit |
| `publication-state.db` | SQLite：本机公开快照的 id、期限、来源摘要、查看密钥和撤销凭据；不保存公开快照明文 |
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
