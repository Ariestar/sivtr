---
title: 配置
description: 创建、查看和编辑 sivtr 配置。
---

`sivtr` 使用 TOML 配置文件。默认路径遵循各平台的配置目录。

## 命令

```bash
sivtr config show
sivtr config init
sivtr config edit
```

| 命令 | 行为 |
| --- | --- |
| `sivtr config show` | 打印配置路径和有效文件内容或默认值 |
| `sivtr config init` | 如果配置不存在，则创建默认配置 |
| `sivtr config edit` | 必要时创建配置，并用配置的编辑器打开 |

## 默认配置

```toml
[general]
open_mode = "tui"
preserve_colors = true

[editor]
command = ""

[history]
auto_save = true
max_entries = 0

[copy]
prompts = []

[codex]
session_dirs = []

[hotkey]
chord = "alt+y"
```

## 默认用编辑器打开

```toml
[general]
open_mode = "editor"

[editor]
command = "nvim"
```

当 `open_mode` 为 `editor` 时，管道模式、run 模式和会话导入会在外部编辑器中打开捕获文本，而不是内置 TUI。

## Prompt 检测

如果你的 prompt 比较特殊，可以添加字面 prompt 前缀：

```toml
[copy]
prompts = ["dev>", "repo $", "PS C:\\repo>"]
```

这有助于命令块解析识别会话日志里的命令输入行。

## 共享 Codex 会话树

当另一个账号发布了只读副本时，可以把共享导出的会话树加入配置：

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

源账号可以这样创建共享树：

```bash
sivtr codex export --dest /srv/sivtr/root-codex
```
