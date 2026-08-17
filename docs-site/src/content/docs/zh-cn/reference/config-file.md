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

## 完整示例

```toml
[editor]
command = "nvim"

[history]
auto_save = true
max_entries = 0

[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]

[hotkey]
chord = "alt+y"
```

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

## codex

```toml
[codex]
session_dirs = []
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `session_dirs` | string array | `[]` | 额外导出的 Codex `sessions` 目录，可通过 `copy codex --pick` 浏览 |

在 macOS 上，典型共享路径是 `/Users/Shared/sivtr/root-codex/sessions`。

目前只有 Codex mirror 在这里配置。其他已注册 provider（Claude、Cursor、OpenCode、OpenClaw、Hermes、Grok、Pi…）使用各自本地位置和环境信号。

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | `sivtr hotkey start` 使用的按键 |
