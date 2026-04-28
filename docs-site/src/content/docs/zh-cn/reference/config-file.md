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

如果存在旧的 `sift/config.toml`，`sivtr` 会为兼容性读取它。

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

[hotkey]
chord = "alt+y"
```

## general

```toml
[general]
open_mode = "tui"
preserve_colors = true
```

| 键 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `open_mode` | `"tui"` 或 `"editor"` | `"tui"` | 捕获输出打开的位置 |
| `preserve_colors` | boolean | `true` | 在 TUI 显示中保留原始 ANSI 颜色 |

## editor

```toml
[editor]
command = "nvim"
```

| 键 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `command` | string | `""` | 编辑器命令。空字符串表示自动检测。 |

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

| 键 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `auto_save` | boolean | `true` | 将捕获输出保存到历史 |
| `max_entries` | integer | `0` | 最多保留条目数。`0` 表示不限。 |

## copy

```toml
[copy]
prompts = []
```

| 键 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `prompts` | string array | `[]` | 检测命令行时使用的 prompt 配置或字面前缀 |

`prompt_presets` 是旧字段，当前配置写入器不会序列化它。

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| 键 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | `sivtr hotkey start` 使用的组合键 |
