---
title: 配置文件
description: TOML 配置参考。
---

## 位置

`sivtr` 使用平台配置目录：

| 平台 | 当前路径 |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

如果存在 legacy `sift/config.toml`，`sivtr` 会为了兼容而读取它。

## 完整示例

```toml
[general]
open_mode = "tui"
preserve_colors = true

[editor]
command = "nvim"

[history]
auto_save = true
max_entries = 0

[copy]
prompts = ["PS C:\\repo> ", "dev>"]

[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]

[hotkey]
chord = "alt+y"
```

## general

```toml
[general]
open_mode = "tui"
preserve_colors = true
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `open_mode` | `"tui"` 或 `"editor"` | `"tui"` | 捕获输出打开位置 |
| `preserve_colors` | boolean | `true` | 在 TUI 显示中保留原始 ANSI 颜色 |

## editor

```toml
[editor]
command = "nvim"
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `command` | string | `""` | 编辑器命令。空值表示自动检测。 |

示例：

```toml
command = "hx"
command = "nvim"
command = "vim"
command = "code --wait"
```

## history

```toml
[history]
auto_save = true
max_entries = 0
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `auto_save` | boolean | `true` | 保存捕获输出到 history |
| `max_entries` | integer | `0` | 最大保留条目数。`0` 表示无限制。 |

## copy

```toml
[copy]
prompts = []
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `prompts` | string array | `[]` | 用于检测命令行的 prompt profile 或字面前缀 |

`prompt_presets` 是 legacy 字段，当前配置写入器不会序列化它。

## codex

```toml
[codex]
session_dirs = []
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `session_dirs` | string array | `[]` | 额外导出的 Codex `sessions` 目录，可通过 `copy codex --pick` 浏览 |

在 macOS 上，典型共享路径是 `/Users/Shared/sivtr/root-codex/sessions`。

目前只有 Codex mirror 在这里配置。Claude、Hermes、OpenCode 和 Pi 使用各自 provider 的本地位置和环境信号发现。

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | `sivtr hotkey start` 使用的按键 |
