---
title: 玩法实例
description: 组合 sivtr 记忆、skill 和 Agent 的社区玩法。
---

玩法实例展示 `sivtr` 在实际场景中如何工作：Agent 使用本地 workspace memory 完成任务。

## 演示

这些短录屏展示核心循环：捕获本地工作、搜索记忆、缩小证据范围，再把精确上下文交给下一条命令或 Agent。

<div class="demo-grid">
  <figure>
    <img src="/demo/1.gif" alt="用 sivtr 搜索最近终端输出" />
    <figcaption>找到最近终端证据。</figcaption>
  </figure>
  <figure>
    <img src="/demo/2.gif" alt="浏览并复用捕获的 workspace memory" />
    <figcaption>浏览并复用已捕获上下文。</figcaption>
  </figure>
  <figure>
    <img src="/demo/3.gif" alt="从本地 Agent 和终端记忆生成时间线" />
    <figcaption>把最近工作变成时间线。</figcaption>
  </figure>
  <figure>
    <img src="/demo/4.gif" alt="在命令链中传递命名记忆变量" />
    <figcaption>把命中结果保存成变量，再继续处理。</figcaption>
  </figure>
</div>

## 玩法目录

| 玩法 | 展示什么 |
| --- | --- |
| [修复最近的终端报错](/zh-cn/playbooks/fix-terminal-error/) | Agent 找到失败原因、修复问题，并验证结果。 |
| [生成最近工作时间线](/zh-cn/playbooks/recent-work-timeline/) | Agent 从时间戳、命令和对话记录中重建你的工作轨迹。 |
| [中断后继续](/zh-cn/playbooks/continue-after-interruption/) | Agent 在猜测"继续"之前先搜索记忆。 |
| [Agent 交接](/zh-cn/playbooks/agent-handoff/) | Agent 准备带证据和下一步的结构化交接文档。 |
| [远程协作记忆](/zh-cn/playbooks/remote-collaboration-memory/) | 未来方向：有权限地搜索队友的 Agent 记忆。 |
