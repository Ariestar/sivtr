---
title: sivtr
description: 面向 AI 时代的终端输出工作区。
---

`sivtr` 把终端输出变成可复用的文本资产。你可以把命令输出管道到浏览器里，包装命令执行，搜索历史捕获，复制结构化命令块，或者直接从当前 Codex 会话里取出最有用的一轮内容，而不用手动打开原始 transcript。

它不是终端模拟器、复用器，也不是替代 shell。它是一个放在现有终端旁边使用的工作台。

## 为什么需要它

终端输出通常是一次性的。命令滚出屏幕后，有用的内容就困在 scrollback、复制模式或巨大的日志里。`sivtr` 给这些输出一个小型工作区：

- 从 stdin、子进程或 shell 集成捕获输出；
- 在 Vim 风格 TUI 里浏览输出；
- 选择字符、行或块范围；
- 用语义选择器复制最近的命令块；
- 使用 SQLite FTS5 搜索已保存的输出；
- 复用当前项目里的 Codex 对话块。

## 第一个命令

从 crates.io 安装，然后把输出管道到 `sivtr`：

```bash
cargo install sivtr
cargo test 2>&1 | sivtr
```

在浏览器里，用 `j` 和 `k` 移动，`/` 搜索，`v` 或 `V` 选择，`y` 复制，`q` 退出。

## 常见工作流

| 目标 | 命令 |
| --- | --- |
| 浏览命令输出 | `cargo test 2>&1 \| sivtr` |
| 运行并捕获命令 | `sivtr run cargo test` |
| 打开当前会话日志 | `sivtr import` |
| 复制最近一次命令输出 | `sivtr copy out` |
| 交互式选择一个或多个块 | `sivtr copy --pick` |
| 复制 Codex 最新回复 | `sivtr copy codex out` |
| 搜索已保存捕获 | `sivtr history search "panic"` |
| 启动 Windows Codex 热键 | `sivtr hotkey start` |

## 文档地图

- 从[安装](/zh-cn/start/installation/)和[快速开始](/zh-cn/start/quickstart/)开始。
- 在[核心概念](/zh-cn/start/core-concepts/)里理解基本模型。
- 在[使用 sivtr](/zh-cn/usage/capture-output/)下查看任务页。
- 在 [CLI 参考](/zh-cn/reference/cli/)里查精确语法。
- 在[架构](/zh-cn/explanation/architecture/)里了解实现结构。
