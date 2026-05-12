---
title: 热键
description: 启动、停止和配置 Windows Codex 选择器热键。
---

热键守护进程目前仅支持 Windows。它注册一个全局快捷键，并打开一个新的终端窗口，为启动守护进程时的工作目录运行 Codex 选择器。

Linux 目前没有内置的桌面级默认 `sivtr` 全局热键。原因是 Wayland
没有给普通 CLI 工具提供统一的跨桌面全局热键接口，而打开 picker
还需要一个交互式终端，这在 GNOME、KDE、Sway、tmux 和纯 SSH
场景之间也没有统一可移植的启动方式。

## 启动

```bash
sivtr hotkey start
```

默认组合键：

```text
alt+y
```

启动时覆盖：

```bash
sivtr hotkey start --chord ctrl+shift+y
```

或在配置中设置：

```toml
[hotkey]
chord = "alt+y"
```

## 查看状态

```bash
sivtr hotkey status
```

状态输出包含：

- 守护进程 pid；
- 组合键；
- 工作目录；
- 可用时的可执行文件路径。

## 停止

```bash
sivtr hotkey stop
```

如果保存的 pid 已失效，`sivtr` 会清理状态文件。

## 行为

按下组合键时，守护进程会启动：

```bash
sivtr hotkey-pick-agent --cwd <daemon-working-directory> --provider all
```

这个内部命令会先打开守护进程工作目录下最新的非空 Codex 会话。如果这个会话不存在或为空，再退回到会话选择器。

普通的 `sivtr copy codex --pick` 不同：它总是从会话选择器开始。

## Linux 设置方式

在 Linux 上，推荐使用以下入口之一，而不是依赖内置全局守护进程：

- VS Code：使用插件默认绑定的 `Alt+Y`。
- tmux：安装 helper 配置块并重新加载 tmux：

```bash
sivtr init tmux
tmux source-file ~/.tmux.conf
```

它会把 `prefix + y` 绑定到当前 pane 的工作目录。

- 桌面快捷键或终端 launcher：为当前项目生成 launcher：

```bash
sivtr init linux-shortcut
```

它会写入 `~/.local/bin/sivtr-pick-codex`，以及
`~/.local/share/applications/sivtr-pick-codex.desktop`。你可以把桌面快捷键绑定到这个脚本，或者直接在终端里运行它。
