---
title: Codex 捕获
description: 从当前 Codex 会话复制有用块。
---

`sivtr copy codex` 会读取 `~/.codex/sessions` 下的 Codex rollout JSONL 文件。默认选择 `cwd` 与当前工作目录匹配的最新会话。

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
sivtr copy codex 2
sivtr copy codex 2..4
sivtr copy codex out 3
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
sivtr copy codex --pick
sivtr copy codex out --pick
```

选择器会显示最近单元，让你选择一个或多个。按 `t` 打开 Vim 风格视图。在 Codex 视图里，如果存在替代完整视图，`T` 可以切换工具内容。

## Windows 热键

在 Windows 上，热键守护进程会为启动它的项目目录打开 Codex 选择器：

```bash
sivtr hotkey start
```

默认组合键是 `alt+y`。可以在 `[hotkey]` 中配置，也可以启动时传 `--chord`。
