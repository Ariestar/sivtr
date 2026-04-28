---
title: 核心概念
description: 捕获、会话、命令块、选择器和输出模式背后的概念。
---

## 捕获

捕获是把终端文本转换成 `sivtr` 可以浏览、搜索、选择或复制的结构化数据。

`sivtr` 支持三条实用捕获路径：

- stdin 管道：`command | sivtr`；
- 子进程包装：`sivtr run <command>`；
- shell 会话 hook：`sivtr init <shell>` 之后正常使用终端。

管道和 run 模式是即时的。Shell 集成会随着使用逐步构建会话日志。

## 会话日志

会话日志是包含命令条目的 JSONL 文件。每条记录保存：

- prompt；
- command；
- output；
- 可选的 ANSI prompt；
- 可选的 ANSI output。

纯文本版本用于稳定复制、搜索和解析。ANSI 版本在可用时保留，这样 `--ansi` 可以保留颜色。

## 命令块

命令块是一条命令的输入和输出：

```text
PS C:\repo> cargo test
running 42 tests
test result: ok
```

`sivtr copy` 可以复制整个块、只复制输入、只复制输出，或者只复制裸命令。

## 选择器

选择器用于选择最近的命令块或 Codex 块。

| 选择器 | 含义 |
| --- | --- |
| `1` | 最新匹配块 |
| `2` | 第二新的匹配块 |
| `2..4` | 最近块的范围 |

选择器按时间新近性解释，因为最常见的任务就是复用刚刚发生的内容。

## 打开模式

捕获的输出可以打开在：

- 内置 TUI 浏览器；
- 外部编辑器。

通过配置控制：

```toml
[general]
open_mode = "tui"
```

或：

```toml
[general]
open_mode = "editor"
```
