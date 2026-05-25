---
title: CLI 参考
description: 命令语法、子命令、选项、provider、selector 和示例。
---

本页记录公开 CLI 表面。事实来源是 `src/cli.rs`；已安装版本请以 `sivtr --help` 和 `sivtr <command> --help` 为准。

## 顶层

```bash
sivtr [COMMAND]
```

不提供命令时，`sivtr` 从 stdin 读取，等价于 pipe mode。

## run

```bash
sivtr run <COMMAND> [ARGS...]
```

运行命令，捕获合并后的 stdout/stderr，报告退出状态，在启用时保存 history，并打开捕获输出。

```bash
sivtr run cargo test
sivtr run git status --short
```

## pipe

```bash
sivtr pipe
```

读取 stdin 并打开。直接管道到 `sivtr` 等价：

```bash
cargo build 2>&1 | sivtr
```

## import

```bash
sivtr import
```

打开当前结构化 shell session log。需要 shell 集成。

## init

```bash
sivtr init <TARGET>
```

支持的 target：

| Target | 用途 |
| --- | --- |
| `powershell` | 安装 Windows PowerShell hook |
| `pwsh` | PowerShell 集成别名 |
| `bash` | 安装 Bash hook |
| `zsh` | 安装 Zsh hook |
| `nushell` / `nu` | 安装 Nushell hook |
| `tmux` | 安装 tmux picker 绑定 |
| `linux-shortcut` | 生成 Linux 桌面/终端 picker launcher |
| `macos-shortcut` | 生成 macOS Terminal/LaunchAgent picker launcher |

## copy

```bash
sivtr copy [MODE] [SELECTOR] [OPTIONS]
```

命令块 mode：

| Mode | 含义 |
| --- | --- |
| 无 mode | 复制输入加输出 |
| `in` | 复制输入 |
| `out` | 复制输出 |
| `cmd` | 复制裸命令 |

别名：

| 别名 | 展开为 |
| --- | --- |
| `sivtr c` | `sivtr copy` |
| `sivtr ci` | `sivtr copy in` |
| `sivtr co` | `sivtr copy out` |
| `sivtr cc` | `sivtr copy cmd` |

通用选项：

| 选项 | 含义 |
| --- | --- |
| `--ansi` | 有可用 ANSI 内容时复制 ANSI-decorated text |
| `--pick` | 打开交互式 picker |
| `--print` | 复制后打印文本 |
| `--regex <PATTERN>` | 只保留匹配正则的行 |
| `--lines <SPEC>` | 只保留 1-based 行选择 |

可复制输入的 mode 还支持：

| 选项 | 含义 |
| --- | --- |
| `--prompt <TEXT>` | 重写复制出来的输入 prompt |

示例：

```bash
sivtr copy
sivtr copy 3 --print
sivtr copy --prompt ":"
sivtr copy in 2..4
sivtr copy out --pick --regex panic
sivtr copy cmd --pick
```

## copy agent provider sessions

```bash
sivtr copy <PROVIDER> [MODE] [SELECTOR] [OPTIONS]
```

Provider：

| Provider | 命令 |
| --- | --- |
| Codex | `sivtr copy codex` |
| Claude Code | `sivtr copy claude` |
| OpenCode | `sivtr copy opencode` |
| Pi | `sivtr copy pi` |

Mode：

| Mode | 含义 |
| --- | --- |
| 无 mode | 最近完整 user + assistant turn |
| `in` | 最近用户消息 |
| `out` | 最近助手回复 |
| `tool` | 最近工具输出 |
| `all` | 完整解析会话 |

Agent copy 选项包含所有通用 copy 选项，外加：

| 选项 | 含义 |
| --- | --- |
| `--session <N|ID>` | 选择第 N 新的可选 session，或匹配 id/id 前缀 |

示例：

```bash
sivtr copy claude
sivtr copy claude out --print
sivtr copy claude --session 2
sivtr copy codex 2..4
sivtr copy codex out --pick
sivtr copy opencode all --lines 1:20
sivtr copy pi tool --regex error
```

## diff

```bash
sivtr diff <LEFT> <RIGHT> [OPTIONS]
```

比较当前 shell session 中两个最近命令块。每个 selector 必须解析成单个块。

内容选项：

| 选项 | 含义 |
| --- | --- |
| `--output` | 比较输出文本，默认值 |
| `--block` | 比较输入加输出 |
| `--input` | 比较带 prompt 的输入 |
| `--cmd` | 比较裸命令文本 |

视图选项：

| 选项 | 含义 |
| --- | --- |
| `--side-by-side` | 显示两列文本视图 |

示例：

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
sivtr diff 2 1 --side-by-side
```

## search

```bash
sivtr search <TARGET> [OPTIONS]
```

搜索捕获到的终端记录和受支持的 AI workspace sessions。Target 决定在哪里搜；filter 决定哪些记录匹配。

Targets：

| Target | 含义 |
| --- | --- |
| `terminal[/<session>[/<record>[/<line>]]]` | 终端命令记录 |
| `agent[/<session>[/<turn>[/<line>]]]` | 所有受支持 AI / Agent 记录 |
| `codex[/<session>[/<turn>[/<line>]]]` | Codex 记录 |
| `claude[/<session>[/<turn>[/<line>]]]` | Claude Code 记录 |
| `opencode[/<session>[/<turn>[/<line>]]]` | OpenCode 记录 |
| `pi[/<session>[/<turn>[/<line>]]]` | Pi 记录 |

可以用 `*` 作为 path segment 通配符，例如 `terminal/*/3` 或 `pi/*/*`。

选项：

| 选项 | 含义 |
| --- | --- |
| `--match <REGEX>` | 大小写不敏感内容过滤 |
| `--exclude <REGEX>` | 大小写不敏感排除过滤，在找到匹配后应用 |
| `--in <FIELD>` | `content`、`title`、`session`、`input`、`output`、`command` 或 `all`；默认是 `content` |
| `--status <STATUS>` | `success`、`failure` 或 `unknown` |
| `--exit-code <CODE>` | 精确终端进程退出码 |
| `--min-duration <DURATION>` | 最小命令持续时间，例如 `500ms`、`2s`、`1m` |
| `--max-duration <DURATION>` | 最大命令持续时间 |
| `--sort <SORT>` | `newest`、`oldest`、`duration`、`duration-asc`、`exit-code` 或 `exit-code-asc` |
| `--cwd <PATH>` | 用于解析记录的 workspace 目录 |
| `--since <TIME>` | 只包含此时间之后或等于此时间的记录 |
| `--until <TIME>` | 只包含此时间之前或等于此时间的记录 |
| `--last <DURATION>` | 最近时间窗口，例如 `30m`、`2h`、`7d` |
| `--latest <N>` | 在最终排序前取最新 N 条匹配记录 |
| `-l, --limit <N>` | 最大打印结果组数 |
| `--exclude-current`、`--other` | Agent 搜索时排除当前 agent session |
| `--json` | `--format json` 的别名 |
| `--format <FORMAT>` | `timeline`、`compact`、`md` 或 `json`；默认是 `json` |

时间过滤支持 RFC3339 时间戳、Unix 秒/毫秒、`30m`、`2h`、`7d` 这样的相对时间，以及 `today`、`yesterday`、`tomorrow`、`this morning`、`this afternoon`、`this evening`、`tonight`、`now` 等别名。

示例：

```bash
sivtr search terminal --status failure --latest 1 --json
sivtr search terminal --match "panic|failed" --exclude "example|sample" --since today --format timeline
sivtr search agent --match "TODO|failed|next step" --since yesterday --format md
sivtr search pi --since today --sort oldest --format timeline
sivtr search pi/019e5941 --match "cargo test" --format compact
sivtr search terminal/session_13104/3/12 --format json
```

## show

```bash
sivtr show <REF> [OPTIONS]
```

打印 Agent provider 或当前终端 session 的 workspace ref。

Ref 语法：

```text
source/session[/dialogue[/line]]
```

选项：

| 选项 | 含义 |
| --- | --- |
| `--cwd <PATH>` | 用于解析 session 的工作区目录 |
| `--json` | 打印机器可读 JSON |

示例：

```bash
sivtr show claude/<session-id>
sivtr show claude/<session-id>/3
sivtr show claude/<session-id>/3/7 --json
sivtr show terminal/current/2
```

## version

```bash
sivtr version [--verbose]
```

打印 Sivtr 版本。使用 `--verbose` 诊断当前运行的是哪个 binary，以及它是否和当前仓库里的本地 debug build 不同。

```bash
sivtr version
sivtr version --verbose
```

Verbose 输出包含：

- package version；
- binary 路径；
- 当前工作目录；
- debug/release profile；
- 可用时的 git commit 和 build time；
- 检测到的 repo root；
- 本地 `target/debug/sivtr` binary 状态；
- 在 repo 内运行不同的全局 binary 时给出 warning。

## history

```bash
sivtr history [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `list [-l, --limit <N>]` | 列出最近条目 |
| `search <KEYWORD> [-l, --limit <N>]` | 搜索保存的捕获 history |
| `show <ID>` | 展示指定 history 条目 |

不提供 history 子命令时，默认使用 `list`。

## config

```bash
sivtr config [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `show` | 显示配置路径和内容 |
| `init` | 创建默认配置 |
| `edit` | 在编辑器中打开配置 |

不提供 config 子命令时，默认使用 `show`。

## hotkey

```bash
sivtr hotkey [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `start [--chord <CHORD>] [--provider <PROVIDER>]` | 启动 Windows 全局热键 daemon |
| `status` | 显示 daemon 状态 |
| `stop` | 停止 daemon |

不提供 hotkey 子命令时，默认使用 `status`。

示例：

```bash
sivtr hotkey start
sivtr hotkey start --chord alt+y
sivtr hotkey start --provider claude
sivtr hotkey status
sivtr hotkey stop
```

## codex export

```bash
sivtr codex export --dest <PATH> [OPTIONS]
```

把本地 Codex rollout JSONL 文件导出到一个包含 `sessions/` 树的目标目录。

选项：

| 选项 | 含义 |
| --- | --- |
| `--dest <PATH>` | 接收 `sessions/` 树的目标目录 |
| `--limit <N>` | 只保留最新 N 个 session 文件；`0` 表示全部导出 |
| `--watch` | 持续 mirror 本地 session |
| `--interval <SECONDS>` | watch 时两次同步之间的秒数；默认 `1` |
| `--interval-ms <MILLISECONDS>` | 两次同步之间的毫秒数；覆盖 `--interval` |

示例：

```bash
sivtr codex export --dest /srv/sivtr/root-codex
sivtr codex export --dest /srv/sivtr/root-codex --watch
sivtr codex export --dest /srv/sivtr/root-codex --limit 100
```

## clear

```bash
sivtr clear [--all]
```

清理当前 shell session log。`--all` 会清理由 `sivtr` 管理的所有记录 session log 和 state 文件。

## 共享语法

Recency selector、`--session`、provider、`--regex`、`--lines`、`--ansi`、`--print` 和 workspace ref 见 [Selector 和 Filter](/zh-cn/reference/selectors-and-filters/)。
