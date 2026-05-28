<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  面向终端输出和 AI Coding Sessions 的本地工作记忆层。
  <br>
  捕获发生过的事，之后搜索复用，让 Agent 使用精确的本地证据。
</p>

<p align="center">
  <a href="https://crates.io/crates/sivtr"><img alt="Crates.io" src="https://img.shields.io/crates/v/sivtr?style=flat-square"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ariestar.sivtr-vscode"><img alt="VS Code Marketplace" src="https://vsmarketplacebadges.dev/version/ariestar.sivtr-vscode.svg?style=flat-square&label=VS%20Code&color=007ACC"></a>
  <a href="https://github.com/Ariestar/sivtr/actions/workflows/rust.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/Ariestar/sivtr/rust.yml?branch=main&style=flat-square"></a>
  <a href="https://deepwiki.com/Ariestar/sivtr"><img alt="Ask DeepWiki" src="https://deepwiki.com/badge.svg?repo=Ariestar/sivtr"></a>
  <a href="rust-toolchain.toml"><img alt="Rust" src="https://img.shields.io/badge/rust-1.88%2B-orange?style=flat-square"></a>
  <a href="https://linux.do/"><img alt="linux.do" src="https://img.shields.io/badge/friend-linux.do-1f883d?style=flat-square"></a>
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

<p align="center">
  <a href="https://sivtr.pages.dev/zh-cn/playbooks/fix-terminal-error/">
    <img src="docs-site/public/demo/1.gif" alt="sivtr demo：查找并复用最近终端输出" width="820">
  </a>
</p>

---

## 为什么需要 sivtr？

开发者和 Agent 经常浪费时间重建已经存在的上下文：终端报错、测试输出、工具日志、之前的 AI 会话。`sivtr` 把这些本地工作变成可搜索的记忆。

有了 `sivtr`，你可以：

- 不再手动把日志粘贴给 Agent；
- 从当前 workspace 同时搜索终端和 AI sessions；
- 复制或打印精确的 records、lines、input/output parts；
- 保存结果集，并在命令链里继续传递。

> [!IMPORTANT]
> Agent 工作流建议同时安装 `sivtr` CLI 和内置 `sivtr-memory` skill。CLI 负责存取本地记忆；skill 负责教 Agent 何时、如何使用它。

## 特性

- **终端捕获**：支持 Bash、Zsh、PowerShell、Nushell。
- **快速 TUI**：浏览长命令输出。
- **统一搜索**：跨 terminal records、Codex、Claude Code、OpenCode、Pi sessions。
- **稳定 refs**：精确定位 records、lines 和 input/output parts。
- **WorkSets**：可复用结果集，支持 `@last`、保存的 `@name` 和 stdin `@`。
- **链式命令**：搜索、缩小、扩展、展示证据，不需要手动复制中间 refs。
- **Agent-ready memory**：通过内置 `sivtr-memory` skill 让 Agent 主动检索本地证据。
- **诊断工具**：`sivtr doctor`、`sivtr init show`、`sivtr init uninstall`。

## 快速开始

安装 CLI：

```bash
cargo install sivtr
```

Linux/macOS 也可以使用预编译安装脚本：

```bash
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

启用 shell capture：

```bash
sivtr init bash       # 或 zsh、powershell、nushell
sivtr doctor
```

捕获并浏览输出：

```bash
cargo test 2>&1 | sivtr
```

搜索最近 workspace memory：

```bash
sivtr s agent -m "TODO|decision|failed" --since today -f timeline
sivtr s terminal --status failure --latest 1 --refs
```

## Agent memory

全局安装内置 skill：

```bash
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

然后让 coding agent 先使用本地记忆：

```text
修复最近的终端报错。先用 sivtr。
```

Agent 可以先搜索本地证据、查看精确 refs、修改代码并验证结果，而不是先要求你粘贴日志。

## 示例

### 复制最近终端输出

```bash
sivtr copy out --print
sivtr copy cmd --pick
```

### 搜索、保存并展示精确证据

```bash
sivtr s terminal -m "panic|failed" --save failures --refs
sivtr work parts @failures --io output --refs
sivtr show @last --full
```

### 链式调用 memory 命令

```bash
sivtr s terminal -m "panic|failed" \
  | sivtr work parts @ --io output \
  | sivtr s @ -m "error|stack|caused by" \
  | sivtr show @ --full
```

### 生成最近工作时间线

```bash
sivtr s agent --since today --sort oldest -f timeline
sivtr s terminal --since today --sort oldest -f timeline
```

### 扩展搜索命中的上下文

```bash
sivtr s agent -m "release|validation" --save release_context --refs
sivtr zoom @release_context -C 2 --save expanded_context
sivtr show @expanded_context --full
```

## 核心概念

| 概念 | 含义 |
| --- | --- |
| WorkRecord | 一个有用的工作事件：终端命令、Agent turn、工具调用或捕获输出块。 |
| WorkPart | Record 里的 typed input/output 片段，例如 command、assistant reply、tool output 或 error。 |
| WorkRef | Record、line 或 part 的稳定地址，例如 `pi/<session>/3/o/1`。 |
| WorkSet | 可以保存、选择、管道传递、扩展和展示的有序结果集。 |

常用句柄：

| 句柄 | 用途 |
| --- | --- |
| `@last` | 最近一次生成的 WorkSet。 |
| `@name` | 通过 `--save name` 保存的 WorkSet。 |
| `@name[1,3..5]` | 从已保存 WorkSet 中按 1-based selector 取子集。 |
| `@` | stdin 传入的 WorkSet JSON。 |

## 命令概览

| 命令 | 用途 |
| --- | --- |
| `sivtr` / `sivtr pipe` | 读取 stdin 并打开输出浏览器。 |
| `sivtr run <command>` | 执行命令、捕获输出并浏览。 |
| `sivtr copy` | 复制最近终端命令块。 |
| `sivtr copy <provider>` | 从 Codex、Claude Code、OpenCode、Pi sessions 复制内容。 |
| `sivtr search` / `sivtr s` | 搜索终端和 Agent memory；结果保存为 `@last`。 |
| `sivtr work sessions` | 列出当前 workspace 的 terminal 和 Agent sessions。 |
| `sivtr work records <source>` | 把 source 或 WorkSet 投影成 record refs。 |
| `sivtr work parts <source>` | 把 records 投影成规范 input/output part refs。 |
| `sivtr show <ref-or-workset>` | 打印精确 refs 或 WorkSets。 |
| `sivtr zoom <source>` | 给 anchors 补上相邻 records。 |
| `sivtr diff <left> <right>` | 对比最近命令块。 |
| `sivtr doctor` | 诊断 binary、config、session logs、hooks、providers、clipboard。 |
| `sivtr init <shell>` | 安装 shell integration；也支持 `show` 和 `uninstall`。 |
| `sivtr config` | 管理 TOML 配置文件。 |
| `sivtr history` | 列出、搜索、查看捕获输出历史。 |
| `sivtr hotkey` | 管理 Windows AI session picker 全局热键守护进程。 |

## 支持来源

| Source | 支持内容 |
| --- | --- |
| Terminal | Bash、Zsh、PowerShell、Nushell shell hooks；pipe 和 run capture。 |
| Codex | 本地 rollout/session JSONL files。 |
| Claude Code | 本地 transcript/session files。 |
| OpenCode | 本地 session data。 |
| Pi | 本地 Pi agent session logs。 |

## 文档

- 文档：[https://sivtr.pages.dev/](https://sivtr.pages.dev/)
- 中文文档：[https://sivtr.pages.dev/zh-cn/](https://sivtr.pages.dev/zh-cn/)
- Playbooks：[https://sivtr.pages.dev/zh-cn/playbooks/](https://sivtr.pages.dev/zh-cn/playbooks/)
- CLI Reference：[docs-site/src/content/docs/reference/cli.md](docs-site/src/content/docs/reference/cli.md)
- Memory skill：[skills/sivtr-memory](skills/sivtr-memory)

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

文档站：

```bash
cd docs-site
bun install --frozen-lockfile
bun run build
```

仓库结构：

```text
crates/sivtr-core/  core model、provider parsers、search、history、config
src/                CLI commands、TUI、shell hooks、hotkey integration
docs-site/          Astro/Starlight documentation site
editors/vscode/     AI session picker 的 VS Code bridge
skills/             bundled agent skills
```
