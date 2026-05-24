---
title: Release Notes
description: sivtr 面向用户的发布说明。
---

`sivtr` 仍处在早期 `0.1.x` 开发阶段。本系列中 CLI 和配置格式仍可能变化。本页总结用户可见变更；仓库中的 `CHANGELOG.md` 仍是更详细的 changelog 来源。

## 0.1.3 - 2026-05-24/25

### Added

- 新增基于 git root 的 workspace scoping，repo 子目录中打开的终端会解析到同一个 workspace。
- 新增 `sivtr init all` / `sivtr init -all`，可一次性安装所有受支持 shell hook。
- 新增 target-first search 语法：`sivtr search terminal|agent|codex|claude|opencode|pi ...`。
- 新增 target path 缩小能力，可定位到 session、record/turn 和 line，例如 `terminal/session_13104/3/12`。
- 新增 search filters：字段（`--in`）、状态、exit code、最小/最大 duration、cwd、时间范围、latest、limit、排除当前 session 和排序。
- 新增本地自然时间别名：`today`、`yesterday`、`tomorrow`、`this morning`、`this afternoon`、`this evening`、`tonight` 和 `now`。
- 新增 search 输出格式：`--format timeline|compact|md|json`。`json` 仍是机器可读默认格式。
- 新增更干净的 search JSON snippet，`matches` 不再输出冗余 `line` 字段。
- 新增 OpenCode 和 Pi 的 Agent search/copy 覆盖，与 Codex、Claude Code 一起作为受支持 provider。
- 新增 `WorkTime`：包含 `started_at`、`ended_at`、`duration_ms`，当能获得其中两个时推导第三个。
- 新增 `sivtr version --verbose`，用于打印 binary 路径、profile、git/build metadata、repo root 和本地 debug binary 诊断。

### Changed

- Search 现在把 target selection 和 filter 分开。旧的 `--scope`、`--provider`、`--recent`、`--json` search flags 已移除，改用 target-first 语法、`--latest` 和 `--format json`。
- Search/show timestamp 统一 normalize 为带 offset 的本地 RFC3339。
- Agent record title 会跳过 `[skill:...]` marker line，优先使用真实用户请求。
- Skill 注入内容会在 record 中压缩，避免 prompt 噪音主导 title 和 search snippet。
- WorkRecord 围绕稳定顶层 `work_ref` 简化，减少重复的 source/id 数据，并保留结构化 text/payload 字段。
- Search 结果按 record/dialogue 分组并带 snippet，减少重复 line 噪音。

### Fixed

- 修复 terminal search 因 PowerShell 本地时间字符串（如 `Mon May 25 00:35:02 2026`）无法解析而返回空结果的问题。
- 修复 interrupted agent turns 无法稳定搜索的问题。
- 修复 record/time/search 重构后的 clippy warnings。

## 0.1.3 - 2026-05-20

### Added

- 新增用于浏览 AI session 的 workspace picker 体验，包括更丰富的内容渲染、搜索导航、滚动和带行号的内容视图。
- 新增 AI session workspace copy 快捷键：`i` 复制用户输入，`o` 复制助手输出，`y` 复制不带 role heading 的完整 dialogue block。
- 新增项目 roadmap 文档页面。

### Fixed

- 加固 VS Code picker command 在 PowerShell、cmd.exe、fish 和 POSIX shells 中的 quoting。
- 忽略 Claude `ai-title` metadata event，避免 session parsing 失败。
- 修复 CI clippy warnings。

## 0.1.2 - 2026-05-02

### Fixed

- 将取消交互式 picker 视为正常退出。

## 0.1.1 - 2026-05-01

### Fixed

- 修复 Codex copy picker TUI 选择逻辑。
- 修复 terminal exit handling，避免终端卡住。

## 0.1.0 - 2026-04-28

### Added

- 新增 `sivtr`：用于捕获命令输出和 AI coding session 的终端输出 workspace。
- 新增 pipe mode：`command | sivtr`。
- 新增 run mode：`sivtr run <command>`。
- 新增 Vim 风格导航、modal interaction、visual selection、搜索和剪贴板复制。
- 新增带 full-text search 的本地 SQLite history。
- 新增 Codex session capture helper：通过 `sivtr copy codex` 复用 assistant reply、user prompt 和 tool output。
- 新增命令块 copy、diff 和 picker 工作流。
- 新增 TOML 配置支持。
- 新增 Windows 全局热键支持，用于 Codex picker 工作流。

### Notes

- 这是第一个公开版本。CLI 和配置格式在 `0.1.x` 系列中仍可能变化。

## 当前文档覆盖范围

当前文档覆盖：

- 终端 pipe 和 run capture；
- shell session logging；
- TUI 浏览和选择；
- 命令块 copy 和 diff；
- Codex、Claude Code、OpenCode 和 Pi 的 AI session copy 与 picker 工作流；
- workspace search 和 show refs；
- SQLite terminal history；
- TOML 配置；
- Windows hotkey、VS Code、tmux、Linux shortcut 和 macOS launcher 流程。
