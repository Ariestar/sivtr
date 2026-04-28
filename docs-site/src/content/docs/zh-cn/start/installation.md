---
title: 安装
description: 使用 Cargo 安装 sivtr 并设置 shell 集成。
---

`sivtr` 已发布为 Cargo 包，项目仓库在 [github.com/Ariestar/sivtr](https://github.com/Ariestar/sivtr)。

## 要求

- Rust 和 Cargo
- 支持的终端
- 当前平台的剪贴板支持

可选：

- `nvim`、`vim` 或 `vi`，用于部分复制工作流里的 Vim 选择视图
- PowerShell、Bash、Zsh 或 Nushell 的 shell profile 权限，用于会话日志

## 使用 Cargo 安装

从 crates.io 安装最新发布版：

```bash
cargo install sivtr
```

验证二进制：

```bash
sivtr --version
sivtr --help
```

## 从源码安装

克隆仓库：

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
```

在仓库根目录安装：

```bash
cargo install --path .
```

## 更新

更新已发布包：

```bash
cargo install sivtr --force
```

或从本地 checkout 拉取后重新安装：

```bash
git pull
cargo install --path . --force
```

Cargo 会替换已安装的二进制。

## Shell 集成

Shell 集成会记录最近的命令块，让 `sivtr copy`、`sivtr import` 和命令块导航有结构化数据可用。

为你的 shell 安装 hook：

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

安装后重启终端。

hook 会写入按进程区分的会话日志：

- Windows PowerShell 和 PowerShell 7 使用 `%APPDATA%\sivtr\session_<pid>.log`。
- Bash 和 Zsh 使用 `$XDG_STATE_HOME/sivtr/session_<pid>.log` 或 `~/.local/state/sivtr/session_<pid>.log`。
- Nushell 使用它的配置目录下的 `sivtr` 会话文件。

## 配置文件

创建默认配置：

```bash
sivtr config init
```

显示路径和当前内容：

```bash
sivtr config show
```

用配置的编辑器打开：

```bash
sivtr config edit
```

完整设置见[配置文件](/zh-cn/reference/config-file/)。
