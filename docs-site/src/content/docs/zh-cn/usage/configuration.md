---
title: 配置
description: 创建、查看、编辑并理解 sivtr 配置。
---

`sivtr` 使用平台配置目录中的 TOML 配置文件。配置控制编辑器交接、history 保留、Codex mirror、TUI 主题、MCP idle 退出、Windows 热键按键和浏览器公开链接 endpoint。

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
[editor]
command = ""

[history]
auto_save = true
max_entries = 0

[codex]
session_dirs = []

[hotkey]
chord = "alt+y"

[theme]
mode = "auto"

[mcp]
idle_exit_secs = 60

[publish]
endpoint = ""
```

`[publish].endpoint` 默认空。创建公开链接前必须写成实际服务地址，例如自建的 `https://share.hnnulwh.cn`。CLI 只打这一个 URL，不会在 Cloudflare Worker 和自建之间自动切换。

字段级说明见[配置文件](/zh-cn/reference/config-file/)。

## History 保留

```toml
[history]
auto_save = true
max_entries = 0
```

`max_entries = 0` 表示无限制。如果不希望 pipe 和 run capture 自动写入 history，设置 `auto_save = false`。

## 共享 Codex session tree

当另一个账号发布只读副本时，添加共享的 Codex session tree：

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

从源账号创建共享树：

```bash
sivtr codex export --dest /srv/sivtr/root-codex
sivtr codex export --dest /srv/sivtr/root-codex --watch
```

目前只有 Codex 有一等共享 mirror 配置。其他 Agent provider 从自己的本地 provider 位置读取。见[数据位置](/zh-cn/reference/data-locations/)。

## 热键按键

```toml
[hotkey]
chord = "alt+y"
```

除非使用 `--chord` 覆盖，否则 `sivtr hotkey start` 会使用这个按键。

Provider 选择是运行时 CLI 选项，不是配置项：

```bash
sivtr hotkey start --provider all
sivtr hotkey start --provider claude
```

## TUI 主题

```toml
[theme]
mode = "auto"
```

`auto` 跟随系统外观，并根据终端能力选择 truecolor 或 ANSI 调色板。用 `dark` 或 `light` 强制配色：

```toml
[theme]
mode = "dark"
```

## MCP 空闲退出

```toml
[mcp]
idle_exit_secs = 60
```

无工具调用多少秒后 stdio MCP server 退出（`0` = 保持到宿主关闭 stdin）。`sivtr mcp serve --idle-exit` flag 可在单次调用时覆盖此值。
