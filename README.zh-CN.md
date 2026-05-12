<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  面向 AI 编程时代的终端输出工作台。
  <br>
  捕获、筛选、浏览、搜索、选择并复用终端输出和 Codex 会话。
</p>

<p align="center">
  <a href="https://crates.io/crates/sivtr"><img alt="Crates.io" src="https://img.shields.io/crates/v/sivtr?style=flat-square"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ariestar.sivtr-vscode"><img alt="VS Code Marketplace" src="https://vsmarketplacebadges.dev/version/ariestar.sivtr-vscode.svg?style=flat-square&label=VS%20Code&color=007ACC"></a>
  <a href="https://github.com/Ariestar/sivtr/actions/workflows/rust.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/Ariestar/sivtr/rust.yml?branch=main&style=flat-square"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square"></a>
  <a href="rust-toolchain.toml"><img alt="Rust" src="https://img.shields.io/badge/rust-1.88%2B-orange?style=flat-square"></a>
</p>

<p align="center">
  <a href="README.md">English</a>
  ·
  <strong>简体中文</strong>
  ·
  <a href="https://sivtr.pages.dev/">Docs</a>
  ·
  <a href="https://sivtr.pages.dev/zh-cn/">中文文档</a>
</p>

---

## sivtr 是什么？

`sivtr` 是一个面向 AI 编程时代的终端输出工作台。它把命令输出、测试失败、构建日志、Codex 会话、工具调用结果变成可以搜索、选择、复制、复用的文本资产。

它不是终端模拟器，也不是 tmux 这类复用器。它更像是你现有终端工作流旁边的“输出处理层”。

## 特性

- 用键盘优先的 TUI 浏览命令输出。
- 把任意命令输出通过管道送入可搜索、可选择的浏览器。
- 记录 shell 命令块，之后复制最近的输入、输出或纯命令。
- 读取 Codex 的 JSONL 会话文件，复制用户消息、助手回复或工具输出。
- 在 VS Code 中用一个快捷键打开 AI session picker。
- 支持用正则和行号范围过滤复制内容。
- 用本地 SQLite 保存历史，之后可以检索。
- 迭代测试和构建时，对比最近几次命令输出。

## 安装

从 crates.io 安装 CLI：

```bash
cargo install sivtr
```

从源码安装：

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
cargo install --path .
```

VS Code 插件：

```text
ariestar.sivtr-vscode
```

插件会从当前 workspace 启动 AI session picker。如果没有安装 `sivtr` CLI，它会提示你是否在可见终端里运行 `cargo install sivtr`。

## 快速开始

浏览命令输出：

```bash
cargo test 2>&1 | sivtr
```

让 `sivtr` 执行命令并捕获输出：

```bash
sivtr run cargo build
```

复制当前 shell 会话中最近的命令块：

```bash
sivtr copy
```

复制当前 Codex 项目会话里的最新助手回复：

```bash
sivtr copy codex out
```

打开 Codex 会话块选择器：

```bash
sivtr copy codex --pick
```

对比最近两次命令输出：

```bash
sivtr diff 1 2
```

## 核心工作流

### 浏览输出

已有命令时用管道：

```bash
some-command --verbose 2>&1 | sivtr
```

希望 `sivtr` 负责执行和捕获时用 run 模式：

```bash
sivtr run cargo test
```

在 TUI 里可以用 Vim 风格按键移动，用 `/` 搜索，用 `v` 进入可视选择，用 `y` 复制。

### 复制命令块

开启 shell 集成后，`sivtr` 会记录命令块，之后可以复制最近输入和输出：

```bash
sivtr copy              # 最近输入 + 输出
sivtr copy out          # 只复制最近输出
sivtr copy in 2..4      # 复制最近第 2 到第 4 个命令的输入
sivtr copy cmd --pick   # 选择并复制纯命令
```

选择器按新到旧编号：`1` 是最新命令块，`2` 是再前一个，`2..4` 代表多个块。

复制后可以再过滤：

```bash
sivtr copy out --regex panic
sivtr copy out --lines 10:40
```

### 复用 Codex 会话

`sivtr copy codex` 会读取 `~/.codex/sessions` 下的 Codex rollout JSONL 文件，并优先选择 `cwd` 与当前目录匹配的最新会话。

```bash
sivtr copy codex        # 最近一轮用户消息 + 助手回复
sivtr copy codex out    # 最近助手回复
sivtr copy codex in     # 最近用户消息
sivtr copy codex tool   # 最近工具输出
sivtr copy codex all    # 整个解析后的会话
```

默认会过滤过程性 commentary，所以 `sivtr copy codex out` 更倾向返回最终助手回复，而不是中间状态更新。

### VS Code 快捷键

VS Code 插件提供命令：

```text
Sivtr: Pick AI Session
```

默认快捷键：

```text
Alt+Y（Linux / Windows）
Cmd+Alt+Y（macOS）
```

你可以改成 `Ctrl+Y`，但它通常会覆盖编辑器的 Redo。

### Linux 桌面快捷键

Linux 没有内置的跨桌面全局 `sivtr` 守护进程。推荐做法是：先准备一个
launcher 脚本，再把它绑定到桌面快捷键。

1. 创建 launcher：

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

2. 把 `~/.local/bin/sivtr-pick-codex` 绑定到桌面快捷键。
   GNOME 路径：`设置 -> 键盘 -> 键盘快捷键 -> 查看和自定义快捷键 -> 自定义快捷键`。
   KDE 路径：`系统设置 -> 快捷键 -> 自定义快捷键`。

3. 按下你设置的组合键（例如 `Ctrl+Alt+Q`）即可打开 picker。

### 其他终端快捷键

如果你不使用桌面级快捷键，可以在终端里绑定原生快捷键执行同一个 launcher
或直接执行命令：

- tmux：`bind-key y new-window -c "#{pane_current_path}" "sivtr copy codex --pick"`
- WezTerm / Kitty / Alacritty / Ghostty：把某个按键绑定为执行
  `~/.local/bin/sivtr-pick-codex`
- 任意终端一次性执行：`sivtr copy codex --pick`

### macOS 快捷方式

这个分支没有新增 macOS 桌面级全局 `sivtr` 守护进程。推荐的 macOS 入口是：

- VS Code：使用插件默认绑定的 `Cmd+Alt+Y`。
- Terminal / iTerm / WezTerm：给某个按键绑定
  `cd <project-path> && sivtr copy codex --pick`。
- 任意终端一次性执行：`sivtr copy codex --pick`

### Windows 全局热键

Windows 上可以启动全局热键守护进程：

```bash
sivtr hotkey start
sivtr hotkey status
sivtr hotkey stop
```

默认快捷键是 `alt+y`。

## 命令速查

| 命令 | 用途 |
| --- | --- |
| `sivtr` / `sivtr pipe` | 从 stdin 读取输出并打开 TUI 浏览器。 |
| `sivtr run <command>` | 执行命令、捕获输出并浏览。 |
| `sivtr copy` | 复制最近命令块。 |
| `sivtr copy codex` | 复制当前 Codex 会话中的有用内容。 |
| `sivtr diff <left> <right>` | 对比最近命令输出。 |
| `sivtr history` | 列出、搜索、查看输出历史。 |
| `sivtr config` | 管理 TOML 配置。 |
| `sivtr init <shell>` | 生成命令块捕获所需的 shell 集成。 |
| `sivtr import` | 打开当前 session log。 |
| `sivtr hotkey` | 管理 Windows AI session picker 热键。 |
| `sivtr clear` | 清空 session logs。 |

## TUI 按键

| 按键 | 模式 | 动作 |
| --- | --- | --- |
| `j` / `Down` | Normal | 下移 |
| `k` / `Up` | Normal | 上移 |
| `h` / `Left` | Normal | 左移 |
| `l` / `Right` | Normal | 右移 |
| `Ctrl-D` | Normal | 下翻半页 |
| `Ctrl-U` | Normal | 上翻半页 |
| `g` | Normal | 到顶部 |
| `G` | Normal | 到底部 |
| `/` | Normal | 搜索 |
| `n` / `N` | Normal | 下一个 / 上一个匹配 |
| `v` / `V` / `Ctrl-V` | Normal | 可视、可视行、可视块 |
| `y` | Visual | 复制选择内容 |
| `Esc` | Visual/Search/Insert | 取消 |
| `q` | Normal | 退出 |

## 文档

- 英文文档：[https://sivtr.pages.dev/](https://sivtr.pages.dev/)
- 中文文档：[https://sivtr.pages.dev/zh-cn/](https://sivtr.pages.dev/zh-cn/)
- VS Code 插件：[editors/vscode/README.md](editors/vscode/README.md)

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

VS Code 插件：

```bash
cd editors/vscode
pnpm install
pnpm run compile
pnpm run package
```

仓库结构：

```text
sivtr/
|- crates/sivtr-core/    # 捕获、解析、缓冲区、选择、搜索、历史、导出
|- src/                  # CLI、TUI、命令、热键集成
|- docs-site/            # Astro/Starlight 文档站
|- editors/vscode/       # AI session picker 的 VS Code 插件桥接
`- .github/workflows/    # CI 和发布自动化
```

## 许可证

sivtr 使用 [Apache License 2.0](LICENSE) 许可证。
