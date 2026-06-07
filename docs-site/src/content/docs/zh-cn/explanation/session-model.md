---
title: Session 模型
description: Shell 命令日志和 Agent provider session 如何变成可复用 memory block。
---

`sivtr` 通过把不同 source 归一化为 memory block 来复用本地工作；这些 block 可以被浏览、搜索、复制，或通过 ref 展示。

```text
source -> session -> block/dialogue -> selector/ref -> copy/search/show/diff
```

## Shell session entry

Shell 集成把命令块记录为结构化 JSONL entry。这给 copy、diff、import 和命令块导航提供了可靠来源。

每个 entry 是一个 `SessionEntry`：

```json
{
  "prompt": "PS C:\\repo> ",
  "command": "cargo test",
  "output": "test result: ok",
  "prompt_ansi": "...",
  "output_ansi": "..."
}
```

当 ANSI 内容和纯文本相同或不可用时，`prompt_ansi` 和 `output_ansi` 会省略。

## Shell 归一化

在构造和加载边界，entry 会被归一化：

- CRLF 转换为 LF；
- 去掉尾部换行；
- 从纯 prompt 和 output 中去掉 ANSI；
- 当 ANSI 内容不同于纯文本时，单独保留 ANSI 内容。

这样既能保证纯文本复制、搜索和解析稳定，也能在 `--ansi` 时使用 ANSI 输出。

## 渲染命令输入

输入部分由 prompt 加 command 渲染。

如果 prompt 以换行结尾，command 会放在下一行。否则 command 追加到 prompt 最后一行。

示例：

```text
PS C:\repo> cargo test
```

多行 prompt：

```text
repo on main
> cargo test
```

## Agent session block

Provider parser 会把本地 transcript 格式转换为共享 `AgentSession` 形状：

```text
AgentSession
|- id
|- cwd
|- title
`- blocks
   |- User
   |- Assistant
   |- ToolCall
   `- ToolOutput
```

这让 copy、picker、search 和 show 逻辑可以跨 Codex、Claude Code、Hermes、OpenCode 和 Pi 工作，而不需要在 UI 中硬编码某一种 transcript 格式。结果是一层 provider-neutral memory，而不是一次性的 transcript reader。

## 当前 workspace 解析

Workspace 命令会相对某个工作目录解析 session：

- provider session 会在可用时按记录的 `cwd` 过滤；
- 当前 shell session log 存在时会加入当前终端 session；
- `search` 和 `show` 可用 `--cwd` 覆盖目录。

`CODEX_THREAD_ID`、`CLAUDE_TRANSCRIPT_PATH`、`CLAUDE_SESSION_ID` 等当前 session 环境提示，可以帮助 provider copy 命令优先选择活跃的本地 transcript。

## 为什么 selector 基于 recency

最常见的 memory 目标是刚刚发生的内容。Recency selector 让它变得便宜：

```bash
sivtr copy out      # 最新输出
sivtr copy out 2    # 上一次输出
sivtr copy 2..4     # 最近多个块
sivtr copy claude 2 # 上一个 Agent turn
```

这避免用户为了临时终端或 agent 工作去记绝对 id。

## Ref 用于精确取回

搜索结果需要稳定的后续 memory 目标时会使用 ref：

```text
source/session[/dialogue[/line]]
```

示例：

```bash
sivtr show claude/<session>/3
sivtr show terminal/current/2
```

Selector 用于最近内容；ref 用于搜索或 picker 工作流返回的精确条目。

## 无效 shell log

如果 shell session log 不能按结构化 entry 解析，`sivtr` 会在追加新 entry 前重置无效 log。这能避免损坏或 legacy 文件破坏正常工作流。

## Legacy 兼容

配置和 history 路径解析会先检查当前 `sivtr` 路径。如果当前文件不存在但 legacy `sift` 文件存在，`sivtr` 会读取 legacy 文件。
