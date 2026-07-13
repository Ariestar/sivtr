<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  面向终端输出和 AI Coding Sessions 的本地工作记忆层。
  <br>
  捕获发生过的事，之后搜索复用，让 Agent 使用精确的本地证据。
  <br>
  <strong>你的 Agent 记忆，不必是一套笨重的知识系统。</strong>
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
  <a href="https://sivtr.pages.dev/zh-cn/playbooks/">
    <img src="docs-site/public/demo/4.gif" alt="sivtr demo：把搜索结果保存成变量并继续缩小范围" width="820">
  </a>
  <br>
  <sub>
    把命中结果保存成记忆变量，再继续缩小范围 ·
    <a href="https://sivtr.pages.dev/zh-cn/playbooks/fix-terminal-error/">修复终端报错</a> ·
    <a href="https://sivtr.pages.dev/zh-cn/playbooks/recent-work-timeline/">生成时间线</a> ·
    <a href="https://sivtr.pages.dev/zh-cn/playbooks/agent-handoff/">带证据交接</a>
  </sub>
</p>

---

## 为什么需要 sivtr？

开发者和 Agent 经常浪费时间重建已经存在的上下文：终端报错、测试输出、工具日志、之前的 AI 会话。`sivtr` 把这些本地工作变成可搜索的记忆，但不要求你引入一套很重的知识系统。

有了 `sivtr`，你可以：

- 让 Agent 修复最近一次失败，而不用自己粘贴日志；
- 几秒钟找回昨天的测试输出、构建报错或关键决策；
- 从摘要跳回当时那条命令输出或 Agent 回复；
- 把一组有用结果保存成 `@failures` 这样的变量，在下一条命令里继续用。

> [!IMPORTANT]
> Agent 工作流建议安装 `sivtr` CLI，用 `sivtr mcp install` 注册 MCP，并可选用内置 `sivtr-memory` skill。MCP 是 Agent 读取本地证据的主路径；skill 负责教它何时、如何调用。

## 特性

- **MCP 优先的 Agent 记忆**：一次 `sivtr mcp install`，Agent 直接调用 `sivtr_search` / `sivtr_show` / `sivtr_zoom` / `sivtr_filter` / `sivtr_status`，不用你粘贴日志。
- **带输出的 shell history**：记录 Bash、Zsh、PowerShell、Nushell 里的命令、stdout/stderr、退出码、目录和耗时。
- **一个搜索面覆盖本地工作**：终端输出 + Codex / Claude Code / Hermes / OpenCode / Cursor / Pi 会话——MCP 或 CLI 都能用。
- **精确证据，而不是摘要**：每个命中都落到稳定 ref，可 show / zoom / filter，或交给下一个 Agent。
- **命名记忆变量**：把结果保存成 `@failures`，复用 `@last`，管道用 `@`，也可 `@failures[1,3..5]` 取子集。
- **跨设备访问**：只读分享 workspace，用 `desk:...` ref 像读本地一样浏览另一台设备。
- **一键安装与诊断**：`sivtr setup` 装 hooks + MCP；`sivtr doctor --fix` 自动修复。
- **人用 CLI 仍然在**：search / show / filter / nav，以及 TUI 浏览器——有用，但不是主叙事。

## 快速开始

安装预编译 CLI（无需 Rust 工具链）：

```bash
cargo binstall sivtr
```

Linux 上 `cargo binstall` 默认安装静态 musl 构建（不依赖系统 GLIBC 版本），与 `install.sh` 同源。

其它方式：

```bash
cargo install sivtr                  # 从源码编译（需要 Rust）
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh   # Linux/macOS/WSL 一行安装
```

Windows（PowerShell）：

```powershell
irm https://raw.githubusercontent.com/Ariestar/sivtr/main/install.ps1 | iex
```

首次安装（hooks + MCP 宿主）：

```bash
sivtr setup             # hooks + MCP 宿主 + sivtr-memory skill（缺失时安装）
# 或分步：
sivtr init powershell   # 或 bash、zsh、nushell
sivtr mcp install       # Claude Code、Cursor、Codex、OpenCode、Pi、Hermes
npx skills add Ariestar/sivtr --skill sivtr-memory -g -y
sivtr doctor
```

> [!NOTE]
> 在 Windows 上，如果 `sivtr init powershell` 提示 profile 没有加载，执行一次 `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` 把当前用户的执行策略调高即可。sivtr 不会修改注册表——hook 只写在你的 PowerShell profile 里。

## Agent 记忆（MCP）

这是主路径。`sivtr mcp install` 之后，Agent 通过结构化工具读写本地终端与 AI session 记忆：

| 工具 | 用途 |
| --- | --- |
| `sivtr_search` | 找最近失败、决策、命令 |
| `sivtr_show` | 打开命中背后的精确 record/part |
| `sivtr_zoom` | 展开前后上下文 |
| `sivtr_filter` | 缩小结果集 |
| `sivtr_status` | workspace / mount / origin 状态 |

可选 skill（教 Agent 何时调用这些工具）：

```bash
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

然后直接说：

```text
修复最近的终端报错。先用 sivtr。
```

Agent 应先搜本地证据、打开原文、改代码并验证——而不是让你粘贴日志。

需要自己查时，CLI 仍然可用：

```bash
sivtr s terminal --status failure --latest 5 --refs
sivtr s agent -m "TODO|decision|failed" --since today -f timeline
```

## 示例

更多完整玩法见 [Playbooks / 玩法实例](https://sivtr.pages.dev/zh-cn/playbooks/)。

| 场景 | 你怎么用 | 演示 |
| --- | --- | --- |
| 修复最近的终端报错 | 对 Agent 说（MCP）：<br><code>修复最近的终端报错。先用 sivtr。</code> | <img src="docs-site/public/demo/1.gif" alt="用 sivtr 修复最近终端报错" width="320"> |
| 中断后继续 | 对 Agent 说：<br><code>继续。先用 sivtr memory。</code> | <img src="docs-site/public/demo/5.gif" alt="中断后用 sivtr 记忆继续" width="320"> |
| 给下一个 Agent 写交接 | 对 Agent 说：<br><code>给下一个 Agent 写一份带证据的交接。</code> | <img src="docs-site/public/demo/6.gif" alt="生成有证据的 Agent 交接" width="320"> |
| 生成最近工作时间线 | <code>sivtr s agent --since today --sort oldest -f timeline</code><br><code>sivtr s terminal --since today --sort oldest -f timeline</code> | <img src="docs-site/public/demo/3.gif" alt="生成最近工作时间线" width="320"> |
| 把结果保存成变量并继续处理 | <code>sivtr s terminal -m "panic" --save failures</code><br><code>sivtr filter @failures --status failure --refs</code> | <img src="docs-site/public/demo/4.gif" alt="链式使用已保存的记忆变量" width="320"> |

## 核心概念

| 概念 | 含义 |
| --- | --- |
| WorkRecord | 一个有用的工作事件：终端命令、Agent turn、工具调用或捕获输出块。 |
| WorkPart | Record 里的命令、输出、assistant 回复、tool output 或 error。只想拿有用片段而不是整个事件时用它。 |
| WorkRef | 某段精确记忆的稳定地址，例如 `pi/<session>/3/o/1`。适合引用、复现和交接。 |
| WorkSet | `@last`、`@failures` 这类记忆变量背后的数据：一组有顺序的 refs，可以筛选、保存、切片、管道传递、导航、扩展和展示。 |

记忆变量：

| 句柄 | 用途 |
| --- | --- |
| `@last` | 最近一次搜索或投影结果。 |
| `@name` | 通过 `--save name` 或 `sivtr var set name` 创建的命名变量，例如 `@failures`。 |
| `@name[1,3..5]` | 从已保存变量中只取几项。 |
| `@` | 使用管道里上一条命令传来的结果。 |

## 命令概览

| 命令 | 用途 |
| --- | --- |
| `sivtr` / `sivtr pipe` | 读取 stdin 并打开输出浏览器。 |
| `sivtr run <command>` | 执行命令、捕获输出并浏览。 |
| `sivtr copy` | 复制最近终端命令块。 |
| `sivtr copy <provider>` | 从 Codex、Claude Code、Hermes、OpenCode、Pi sessions 复制内容。 |
| `sivtr search` / `sivtr s` | 搜索终端和 Agent memory；命中结果保存为 `@last`。 |
| `sivtr filter <source>` | 对 source 或管道传入的 WorkSet 应用统一过滤。 |
| `sivtr var` | 列出、保存、删除、合并、移除或清空命名 WorkSet 变量。 |
| `sivtr nav <source> <motion>` | 用 `<`、`>N`、`+N`、`-N`、`[A..B]`、`~` 确定性移动 anchors。 |
| `sivtr work sessions` | 列出当前 workspace 的 terminal 和 Agent sessions。 |
| `sivtr work records <source>` | 把 sessions 或已保存变量转成事件级 refs。 |
| `sivtr work parts <source>` | 从匹配事件里抽出真正有用的输入/输出片段。 |
| `sivtr show <ref-or-workset>` | 打印 refs、`@last`、`@name` 或管道结果背后的内容。也支持远程 ref，如 `desk:terminal/...`。 |
| `sivtr zoom <source>` | 给搜索命中补上前后 record 上下文。 |
| `sivtr diff <left> <right>` | 对比最近命令块。 |
| `sivtr serve` | 启动/停止本机 remote-memory daemon。 |
| `sivtr share` | 显式分享本机 workspace 给远端。 |
| `sivtr remote` | 把远端 share 挂到当前 workspace（`add`/`list`/`remove`/`test`）。 |
| `sivtr workspace` / `sivtr ws` | 列出本机已知 workspace（`name:body` 的 origin 标签）。 |
| `sivtr mcp` | MCP server 与宿主安装（`serve` / `install` / `uninstall` / `print-config`）。 |
| `sivtr doctor` | 诊断 binary、config、session logs、hooks、providers、clipboard。 |
| `sivtr init <shell>` | 安装 shell integration；也支持 `show` 和 `uninstall`。 |
| `sivtr config` | 管理 TOML 配置文件。 |
| `sivtr history` | 列出、搜索、查看捕获输出历史。 |
| `sivtr hotkey` | 管理 Windows AI session picker 全局热键守护进程。 |

## 远程访问

两台装了 sivtr 的设备可以像读本地一样互相读取 workspace 的 session——用于协同开发：想看队友的终端输出或 AI 会话时，不用离开自己的机器。

ref 统一为 `origin:body`：

```text
codex/4                 # 本机当前 workspace
docs:codex/4            # 本机另一个 workspace（按目录名）
desk:terminal/...       # 已挂载的远端别名
alice/sivtr:hermes/...  # device/workspace 坐标
```

在持有 workspace 的设备上：

```bash
sivtr share                   # 交互选择 workspace（Enter = 当前），打印 invite key
sivtr ws list                 # 查看本机 workspace origin 标签
```

在另一台设备上：

```bash
sivtr remote add desk <invite-key>   # 直接粘贴 `sivtr share` 输出的 bare key
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr show desk:terminal/session_42/3/o/1
sivtr zoom desk:terminal/session_42/3 -C 2
sivtr nav desk:terminal/session_42/3 +1 --refs
sivtr copy ref desk:terminal/session_42/3/o/1 --print
```

分享是 opt-in、只读，默认在数据离开本机前脱敏常见密钥。远程传输走加密 iroh；需要时会自动启动 daemon。未登记的 origin 会报错——用 `sivtr remote add` 挂载，或用 `sivtr ws` 查看本机 workspace。

## 支持来源

| Source | 支持内容 |
| --- | --- |
| Terminal | Bash、Zsh、PowerShell、Nushell shell hooks；pipe 和 run capture。 |
| Codex | 本地 rollout/session JSONL files。 |
| Claude Code | 本地 transcript/session files。 |
| Hermes | 本地 Hermes session JSONL files。 |
| OpenCode | 本地 session data。 |
| Pi | 本地 Pi agent session logs。 |

## 文档

- 文档：[https://sivtr.pages.dev/](https://sivtr.pages.dev/)
- 中文文档：[https://sivtr.pages.dev/zh-cn/](https://sivtr.pages.dev/zh-cn/)
- Playbooks：[https://sivtr.pages.dev/zh-cn/playbooks/](https://sivtr.pages.dev/zh-cn/playbooks/)
- CLI Reference：[docs-site/src/content/docs/reference/cli.md](docs-site/src/content/docs/reference/cli.md)
- Memory skill：[skills/sivtr-memory](skills/sivtr-memory)

## 开发

环境、PR 约定与编码指南见 [CONTRIBUTING.md](CONTRIBUTING.md)。

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
