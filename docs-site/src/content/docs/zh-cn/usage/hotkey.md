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
sivtr hotkey-pick-codex --cwd <daemon-working-directory>
```

这个内部命令打开的选择流程等同于：

```bash
sivtr copy codex --pick
```
