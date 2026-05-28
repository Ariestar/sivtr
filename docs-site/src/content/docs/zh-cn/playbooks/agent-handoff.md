---
title: Agent 交接
description: 为下一个人或 Agent 准备简洁、有证据的交接文档。
---

## 演示

<img src="/demo/6.gif" alt="用 sivtr 准备带证据的 Agent 交接" class="demo-wide" />

## 场景

你今天做完了，或者要换一个 Agent 继续。你希望下一个接手的人或 Agent 能精确地从你停下的地方继续——目标、决策、验证结果、下一步全部记录在案。

## 你只需要说

```text
给下一个 Agent 写交接。
```

## 处理过程

Agent 收集最近的决策和验证证据：

```bash
sivtr search agent --match "current goal|next step|TODO|blocked|decision|test result|commit|passed|failed" --since today --format json --limit 30
sivtr search terminal --since today --sort oldest --format timeline --limit 20
sivtr copy cmd 1..10 --print
```

然后生成结构化的交接文档。

## 输出示例

```text
## 目标
把 docs-site 迁移到 Bun-only，并补充 skills 文档。

## 当前状态
两个任务都已完成。构建通过，生成 69 页。

## 验证
- `bun run check`：0 错误，0 警告，0 提示。
- `bun run build`：69 页构建成功。
- VS Code 插件：8 个测试通过，vsix 已打包。

## 已做的决策
- Playbooks 拆分为独立顶层 section，方便后续挂视频演示。
- 远程协作保留为 roadmap 方向，暂不实现。

## 风险
- Sitemap 警告（非阻塞，需要在 astro.config.mjs 中配置 `site`）。

## 下一步
- 为 playbook 页面录制演示视频。
- 提交并推送所有文档变更。
```

## 视频演示大纲

这个玩法适合较长视频或直播，展示工作如何跨多个 Agent 会话延续。
