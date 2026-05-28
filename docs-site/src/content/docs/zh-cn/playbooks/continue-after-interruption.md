---
title: 中断后继续
description: Agent 先搜索本地记忆，再判断如何继续。
---

## 演示

<img src="/demo/5.gif" alt="中断后用 sivtr 记忆继续工作" class="demo-wide" />

## 场景

你的会话被中断了——可能是上下文被压缩，可能是你切换了任务。你回来说"继续"，`sivtr` 可以帮助 Agent 从本地记忆中找到最近的工作线索。

## 你只需要说

```text
继续。
```

## 处理过程

Agent 搜索最近的目标、决策和验证标记：

```bash
sivtr search agent --match "next step|TODO|blocked|decision|test result|passed|failed" --since today --format json --limit 20
sivtr search terminal --since today --sort oldest --format timeline --limit 20
```

如果只有一条明确主线，直接继续。如果有多个可能任务，先让你选择。

## 交互示例

```text
用户：继续。

Agent：我在 sivtr 记忆中找到两条近期线索：
1. Skills 文档（最后活动：添加了 playbooks section，build 通过）。
2. VS Code 插件打包（最后活动：测试通过，vsix 已构建）。
你要继续哪一个？
```

## 视频演示大纲

1. 做一个任务，推进到一定程度。
2. 开启一个新的 Agent 会话（模拟中断）。
3. 只说"继续"。
4. 展示 Agent 从记忆中重建上下文。
