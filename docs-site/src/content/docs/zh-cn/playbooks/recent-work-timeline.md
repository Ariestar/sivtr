---
title: 生成最近工作时间线
description: 把终端和 Agent 记忆变成有证据支撑的工作时间线。
---

## 场景

你想回顾最近做了什么，但不想要泛泛的总结——你要的是有时间戳、有命令输出、有 Agent 对话记录支撑的时间线。

## 你只需要说

```text
按时间线总结一下我最近做了什么。
```

## 处理过程

Agent 可以先直接渲染 timeline 视图：

```bash
sivtr search agent --since today --sort oldest --format timeline
sivtr search terminal --since today --sort oldest --format timeline
```

如果需要更窄的证据，再搜索计划、变更和验证标记：

```bash
sivtr search agent --match "TODO|next step|decision|changed|fixed|test result|passed|failed|commit|build" --since today --format json --limit 50
```

再查看最近命令标题：

```bash
sivtr copy cmd 1..20 --print
```

然后组装时间线，每条记录都有时间戳、活动标题、证据来源和结果。

## 输出示例

```text
1. 10:12 — 排查文档构建失败
   证据：terminal/current/4（Astro check 因缺少页面失败）
   结果：修复 sidebar 链接，重新运行 check 通过。

2. 10:43 — 把产品文档重构为 agent memory 叙事
   证据：claude/<session>/7（讨论了新的定位）
   结果：文档已更新，build 通过。

3. 11:15 — 把 docs-site 从 pnpm 迁移到 Bun
   证据：terminal/current/9（bun install + bun run build）
   结果：57 页构建成功，所有检查通过。
```

每条记录都能追溯到具体的命令输出或 Agent 对话。

## 视频演示大纲

展示 Agent 如何从 `sivtr` 记忆中生成有来源、有时间戳、有验证的时间线。
