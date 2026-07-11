---
title: Roadmap
description: sivtr 和更广义 Agent Memory Workspace 的方向性产品路线图。
---

这份 roadmap 是工作计划,不是发布承诺。它用结果导向描述 `sivtr` 的方向:保持一个小而有用的终端工具,同时逐步成为面向人和 Agent 的统一 Agent Memory Workspace。

## Roadmap map

```text
Reliable CLI
  -> Multi-agent workspace
    -> Skills and playbooks
      -> High-signal TUI
        -> Remote collaboration
          -> sivtr-me
```

| Track | 状态 | 目标结果 |
| --- | --- | --- |
| CLI foundation | 进行中 | 一个日常可用的 CLI,用于捕获、搜索、选择和导出终端与 agent 工作。 |
| Agent support | 进行中 | 面向 AI Agent 对话记录的 provider-neutral 解析和浏览。 |
| Skills and playbooks | 进行中 | 把 `sivtr` 作为统一记忆入口的可复用 Agent 流程。 |
| TUI workspace | 规划中 | 面向多 session、多 provider、长对话的高密度键盘优先界面。 |
| Remote collaboration | 核心已落地 | 通过 Share / Grant / Mount 有权限地只读访问队友 workspace 记忆。 |
| `sivtr-me` | 更后期 | 基于真实工作记录生成、可追溯证据支撑的个人 AI 时代 profile。 |

## CLI foundation

近期优先级是让命令行表面完整、可预测、可脚本化。在成为更大的个人数据层之前,`sivtr` 必须先是可靠的日常工具。

- [x] 从 pipe mode 捕获命令输出。
- [x] 用 `sivtr run` 捕获子进程输出。
- [x] 导入 shell session log。
- [x] 按 selector 复制最近命令输入、输出和命令块。
- [x] 用 SQLite 搜索保存过的输出 history。
- [x] 为核心行为提供 TOML 配置。
- [ ] 收紧 `copy`、`history`、`codex`、`hotkey` 和 workspace flows 的命名与选项一致性。
- [ ] 让 selector 和 filter 更容易在 shell 脚本中组合。
- [ ] 扩展搜索能力:明确 scope、literal/keyword/fuzzy/semantic 方法、source filter、ranking 和上下文丰富的机器可读结果。
- [ ] 强化大型本地 archive 的 import、export 和 search 行为。
- [ ] 保持配置显式、可移植、适合安全共享。

## Agent support

Agent session 是一等 memory source。产品目标是让 Agent transcript 像普通 `sivtr` source 一样工作,而不是特殊功能。

- [x] 解析 Codex session 记录。
- [x] 解析 Claude-style session 记录。
- [x] 解析 Hermes、OpenCode 和 Pi session 记录。
- [x] 复制最新 user、assistant、tool、turn 或完整 session block。
- [x] 通过 picker 浏览本地和镜像 session 目录。
- [ ] 在共享 session-provider 接口后支持更多 agent provider。
- [ ] 让 provider-specific parsing 与共享 selection、search、export 逻辑保持隔离。
- [ ] 让 session discovery 在本地、镜像和共享 transcript 目录中更加稳健。
- [ ] 在 CLI 命令、hotkey 和 TUI workspace 中一致暴露 provider selection。
- [ ] 避免把数据模型绑定到单一 vendor 的 transcript 格式。

## Skills and playbooks

Skill 让 Agent 可以把 `sivtr` 当成共享记忆入口。它们把通用 memory 命令变成可复用流程，例如"修复最近的终端报错""从上次任务继续""按时间线总结最近工作"。

- [x] 增加初始 `skills/sivtr-memory/` 包，包含命令配方、证据纪律、工作流和示例。
- [x] 在文档中说明 skill 是产品模型的一部分，而不只是可选 prompt 片段。
- [ ] 定义社区 skill 和团队 playbook 的稳定打包约定。
- [ ] 建立 skill registry，让用户发现终端失败调试、timeline 生成、PR handoff、recap、onboarding 等 workflow。
- [ ] 增加示例，展示 Agent 如何使用 ref 和验证证据。
- [ ] 保持 skill procedure 基于现有 CLI 命令，避免社区玩法暗示还不存在的 `sivtr` 功能。

## TUI workspace

TUI 应保持快速和键盘优先,但需要从单个输出浏览扩展到多 source workspace 导航。

- [x] 在 Vim 风格终端 UI 中浏览捕获输出。
- [x] 搜索捕获输出。
- [x] 选择字符、行和块范围。
- [x] 交互式选择 session 和 dialogue block。
- [ ] 优化大量 session、provider 和长对话场景下的 workspace picker。
- [ ] 改进搜索 scope、结果导航和视觉反馈。
- [ ] 统一终端输出、命令块和 Agent dialogue block 的选择行为。
- [ ] 改进 markdown、tool call 和结构化 agent content 的渲染。
- [ ] 保持界面高密度、可预测、editor-friendly。

## Remote collaboration

远程协作把 local memory 模型扩展到有权限的队友记录。目标不是变成托管 transcript 服务，而是让明确授权的协作者挂载相关 workspace 记忆，使 Agent 能跨机器协作。

核心模型已落地：**Device Daemon + Identity + Share + Grant + Mount**，传输走加密 iroh。ref 使用 `origin:body`（如 `desk:terminal/...`）。

- [x] 设备级 daemon，可自动启动（`sivtr serve`）。
- [x] 显式 workspace 分享（`sivtr share` / `share add` / `invite` / `grants` / `revoke`）。
- [x] workspace 本地 mount（`sivtr remote add|list|remove|rename|test`）。
- [x] peer 身份 list/forget（`sivtr peer`）。
- [x] WorkRef 支持 remote origin（`origin:body`），可用于 search / show / filter / nav / zoom / copy。
- [x] 数据离开本机前默认脱敏。
- [x] 本机 workspace origin 标签（`sivtr wb list`）。
- [ ] Identity CLI（`identity show|rotate|export`）。
- [ ] share audit log 与按 share 的 redact 开关 CLI。
- [ ] 登录时自动启动 daemon（`serve enable|disable`）。
- [ ] peer rename / verify / disconnect 辅助命令。
- [ ] 用 UDS 或 named-pipe 替换 localhost TCP 控制面。
- [ ] 旧服务端协议版本协商。
- [ ] 更细的 selective disclosure（按 session 分享，而不是整个 workspace）。

## sivtr-me

当 CLI 和 workspace foundation 稳定后,更大的方向是 `sivtr-me`:从累积工作记录生成个人 profile。它不像静态简历,而是持续从真实 terminal session、Agent conversation、project history 和选中 artifact 中更新,并由证据支撑。

- [ ] 定义长期个人工作记录的本地数据模型。
- [ ] 从真实记录总结项目、工具、领域和工作方式。
- [ ] 展示代表性的 conversation、decision、code change、debug trace 和 shipped outcome。
- [ ] 构建可公开或私有的 profile,用于回答"这个人实际做过什么?"
- [ ] 支持 selective disclosure,让敏感记录保持本地,同时共享高信号 summary。
- [ ] 为每个展示 claim 保留到源 session 或 artifact 的 provenance。

## Non-goals

Roadmap 不表示 `sivtr` 会变成:

- 终端模拟器;
- 默认托管 transcript storage 服务；
- 没有明确权限的远程 chat 镜像；
- 某一个 AI assistant 的 vendor-specific wrapper;
- source control、issue tracker 或笔记工具的替代品。

`sivtr` 应该在边缘保持小,在核心保持结构化。

## Principles

- **Capture first.** 重要工作应该在发生时记录,而不是事后凭记忆重建。
- **Local by default.** 个人 transcript 和 terminal history 应由用户控制,除非显式分享或导出。
- **Provider-neutral.** Agent support 应通过可替换 provider 和稳定共享抽象实现。
- **Skills are interfaces.** Skill 是 Agent 学会操作共享记忆层的方式；它应该精确、可验证、以证据为先。
- **Composable CLI.** 在可行时，每个交互特性都应有脚本化路径。
- **Provenance matters.** Summary、profile 和 export 应能追溯到源 session 和命令输出。
- **Editor-friendly.** `sivtr` 应交给已有编辑器和工作流,而不是试图拥有整个开发环境。
