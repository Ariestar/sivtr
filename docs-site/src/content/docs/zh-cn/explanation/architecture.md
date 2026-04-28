---
title: 架构
description: sivtr workspace 如何拆分 CLI、TUI 和核心模块。
---

`sivtr` 是一个 Cargo workspace，主要分两层：

- `sivtr`：位于 `src/` 的二进制 crate；
- `sivtr-core`：位于 `crates/sivtr-core/` 的库 crate。

二进制层负责用户交互：CLI 解析、命令分发、TUI 状态和平台相关热键行为。核心 crate 负责可复用逻辑：捕获、解析、buffer、选择、搜索、历史、导出、配置和 Codex 会话解析。

## Workspace 布局

```text
sivtr/
|- Cargo.toml
|- src/
|  |- cli.rs
|  |- main.rs
|  |- app.rs
|  |- commands/
|  `- tui/
`- crates/
   `- sivtr-core/
      `- src/
         |- buffer/
         |- capture/
         |- config/
         |- export/
         |- history/
         |- parse/
         |- search/
         |- selection/
         |- session/
         `- codex.rs
```

## 二进制 crate

二进制 crate 包含：

| 区域 | 职责 |
| --- | --- |
| `cli.rs` | clap 命令定义和帮助文本 |
| `commands/` | run、pipe、copy、history、config、hotkey、diff 和 import 的命令处理 |
| `app.rs` | TUI 状态机 |
| `tui/` | 终端设置、事件处理和渲染 |
| `command_blocks.rs` | 用于会话浏览和复制的命令块 span |

这一层可以依赖终端 UI 库、平台 API 和进程启动行为。

## 核心 crate

核心 crate 包含可复用领域逻辑：

| 模块 | 职责 |
| --- | --- |
| `capture` | stdin、子进程和 scrollback/session 捕获辅助 |
| `parse` | ANSI 去除、Unicode 显示宽度和行解析 |
| `buffer` | 行、光标和 viewport 模型 |
| `selection` | visual、line 和 block 选择提取 |
| `search` | 匹配和导航状态 |
| `history` | SQLite 存储、schema 和搜索 |
| `export` | 剪贴板、文件和编辑器导出辅助 |
| `config` | TOML 配置模型、默认值和路径解析 |
| `session` | 结构化会话条目和渲染 |
| `codex` | Codex 会话发现、解析和格式化 |

这个拆分让计算和数据处理能独立于 TUI 进行测试。

## 捕获流程

管道模式：

```text
stdin -> capture::pipe -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Run 模式：

```text
subprocess -> combined output -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

会话导入：

```text
session log -> render entries -> parse::parse_lines -> Buffer -> command block spans -> TUI/editor
```

复制模式：

```text
session log -> SessionEntry list -> command blocks -> selector -> filters -> clipboard
```

Codex 复制：

```text
~/.codex/sessions -> current cwd match -> parsed blocks -> selector -> filters -> clipboard
```

## 设计边界

前端层负责展示和交互。Rust 核心负责计算：解析、捕获、选择提取、搜索、存储和格式化。这样 UI 改动不会泄漏进数据模型，命令行为也更容易测试。
