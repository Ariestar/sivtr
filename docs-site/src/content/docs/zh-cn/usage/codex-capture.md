---
title: Codex 捕获
description: 从当前 Codex 会话复制有用块。
---

`sivtr copy codex` 会读取 `~/.codex/sessions` 下的 Codex rollout JSONL 文件。如果当前 shell 暴露了 `CODEX_THREAD_ID`，`sivtr` 会优先匹配这个本地精确会话；否则默认选择 `cwd` 与当前工作目录匹配的最新本地会话。

如果另一个账号先用 `sivtr codex export --dest ...` 发布了只读镜像，就把镜像后的 `sessions` 目录加入 `[codex].session_dirs`。这样显式 `--pick` 浏览就能在不提权的前提下读取它。

用 `--session N` 可以显式选择第 N 新的已记录会话；用 `--session ID` 可以按会话 id 或 id 前缀匹配。

当你想复用最后一个回答、输入、工具输出，或整个解析后的会话，但不想手动打开 Codex transcript 时，这个功能很有用。

## 默认行为

```bash
sivtr copy codex
```

默认复制最近一个已完成的用户消息加助手回复。

## 复制特定类型

```bash
sivtr copy codex out
sivtr copy codex in
sivtr copy codex tool
sivtr copy codex all
```

| 命令 | 复制内容 |
| --- | --- |
| `sivtr copy codex` | 最近用户消息加助手回复 |
| `sivtr copy codex out` | 最近助手回复 |
| `sivtr copy codex in` | 最近用户消息 |
| `sivtr copy codex tool` | 最近工具输出 |
| `sivtr copy codex all` | 整个解析后的会话 |

## 选择更早的内容

选择器和命令块复制相同：

```bash
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy codex 2
sivtr copy codex 2..4
sivtr copy codex out --session 3
```

`1` 表示最新匹配的 Codex 单元，`2` 表示第二新，依此类推。

## 过滤 Codex 文本

```bash
sivtr copy codex tool --regex error
sivtr copy codex all --lines 1:40
```

用 `--print` 检查复制的文本：

```bash
sivtr copy codex out --print
```

## 交互式选择

```bash
sivtr copy codex --session 2 --pick
sivtr copy codex --pick
sivtr copy codex out --pick
sivtr copy codex --pick  # 同时浏览 [codex].session_dirs 里的共享镜像会话
```

普通 CLI 选择器会先显示会话列表，进入某个会话后再选择一个或多个单元。按 `t` 打开 Vim 风格视图。在 Codex 视图里，如果存在替代完整视图，`T` 可以切换工具内容。

在 Codex 选择器里，按 `m` 只复制当前高亮对话 `## User` 的正文。输入 `2:8m`
这样的 1-based 行范围，可以只复制该 `## User` 正文里的第 2 到第 8 行。
复制结果不会包含 `## User` 和 `## Assistant` 这两行标题。

Windows 热键和 VS Code 插件这类带上下文的入口会先打开当前 workspace 下最新的非空会话。如果这个会话不存在或为空，再退回到会话列表。

共享/镜像会话树只参与显式 `--pick` 浏览。隐式“当前会话”解析仍然只看本地会话，避免另一个账号导出的历史抢占当前用户的 Codex 工作流。

## 为另一个账号镜像会话

先在源账号侧持续发布镜像树：

```bash
sivtr codex export --dest /srv/sivtr/root-codex --watch
```

再在另一个账号的配置里引用镜像目录：

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

在 macOS 上，`/Users/Shared/sivtr/root-codex` 很适合作为不同本地账号之间
共享的只读目录：

```bash
sivtr codex export --dest /Users/Shared/sivtr/root-codex --watch
```

```toml
[codex]
session_dirs = ["/Users/Shared/sivtr/root-codex/sessions"]
```

可直接复制的一行验证命令：

- 导出侧：`rm -rf /Users/Shared/sivtr/root-codex-smoke && sivtr codex export --dest /Users/Shared/sivtr/root-codex-smoke && find /Users/Shared/sivtr/root-codex-smoke -maxdepth 2 -type f | sed -n '1,5p'`
- 读取侧（在 `[codex].session_dirs` 配好之后）：`sivtr copy codex --pick`

## Windows 热键

在 Windows 上，热键守护进程会为启动它的项目目录打开 Codex 选择器：

```bash
sivtr hotkey start
```

默认组合键是 `alt+y`。可以在 `[hotkey]` 中配置，也可以启动时传 `--chord`。
