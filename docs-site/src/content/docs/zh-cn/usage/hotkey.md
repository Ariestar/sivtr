---
title: 热键
description: 启动、停止和配置 Windows Codex 选择器热键。
---

热键守护进程目前仅支持 Windows。它注册一个全局快捷键，并打开一个新的终端窗口，为启动守护进程时的工作目录运行 Codex 选择器。

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

## Linux 桌面快捷键（手动配置）

Linux 在 GNOME/KDE/Wayland/X11 之间没有统一的 CLI 全局热键 API。
推荐使用 launcher 脚本 + 桌面快捷键绑定。

1. 创建 launcher 脚本：

```bash
mkdir -p ~/.local/bin
cat > ~/.local/bin/sivtr-pick-codex <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
export PROJECT_CWD="$HOME"
# 可选：跨账号会话镜像目录
# export SIVTR_CODEX_SESSION_DIRS='/srv/sivtr/root-codex/sessions:/home/<user>/codex_transfer/sessions'
exec x-terminal-emulator -e bash -lc 'cd "$PROJECT_CWD"; exec sivtr copy codex --pick'
EOF
chmod +x ~/.local/bin/sivtr-pick-codex
```

2. 把 `~/.local/bin/sivtr-pick-codex` 绑定到快捷键。
   GNOME 路径：`设置 -> 键盘 -> 键盘快捷键 -> 查看和自定义快捷键 -> 自定义快捷键`。
   KDE 路径：`系统设置 -> 快捷键 -> 自定义快捷键`。

3. 按下组合键（例如 `Ctrl+Alt+Q`）即可打开 picker。

## 其他终端快捷键

如果你更偏好终端内快捷键：

- tmux：

```tmux
bind-key y new-window -c "#{pane_current_path}" "sivtr copy codex --pick"
```

- WezTerm / Kitty / Alacritty / Ghostty：将某个按键绑定为执行
  `~/.local/bin/sivtr-pick-codex`。
- 任意终端一次性命令：

```bash
sivtr copy codex --pick
```

## macOS 快捷方式

这个分支没有新增 macOS 桌面级 `sivtr` 守护进程。推荐的 macOS 入口是：

- VS Code：使用插件默认绑定的 `Cmd+Alt+Y`。
- Terminal / iTerm / WezTerm：给某个按键绑定
  `cd <project-path> && sivtr copy codex --pick`。
- 任意终端一次性命令：

```bash
sivtr copy codex --pick
```
