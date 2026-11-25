---
title: MCP 跨会话任务恢复
description: 通过 MCP 搜索、命名 WorkSet 和带引用的证据，继续正确的任务。
---

## 场景

你新开一个 Agent 会话，说「继续 retry-policy」。最新记录属于另一个任务，更早的 retry-policy 记录提出了不同的重试上限。只拿最新记录会接到错误的线程。

## 你说什么

```text
继续 retry-policy。不要用最新记录；引用已保存的决策和检查结果。
```

## 怎么做

Agent 走 `sivtr mcp serve` 工具，不是 CLI，也不把 `@last` 当成当前任务：

1. `sivtr_search` 用任务过滤（`query` / `match_regex`），排除更新但不相关的记录。
2. `sivtr_filter`（或 `save`）留下当前线程，存成命名 WorkSet，例如 `@current`。下一次搜索会覆盖 `@last`。
3. MCP 进程重启后（新会话或 idle-exit），用 `sivtr_show` 展开 `@current[n]`，不要用 `@last`。

## 示例工具参数

```text
sivtr_search  source=@fixture  query=retry-policy  match_regex=retry-policy  save=recovery
sivtr_filter  source=@recovery  since=<当前线程的时间窗>  save=current
sivtr_show    source=@current[1]  mode=full
```

`@current[1]` 这类索引对该已保存选择是 1-based。使用正文前，把展开的 `contents[].ref` 和返回的 anchor 对上。

## 为什么不能只看最新

| 记录 | 时间（UTC） | 角色 |
| --- | --- | --- |
| `codex/synthetic-retry-policy-old/1` | 2026-01-01 10:00 | 过期失败检查；提出上限 5。 |
| `codex/synthetic-retry-policy/1` | 2026-01-02 10:00 | 决策：上限 2，保留指数退避。 |
| `codex/synthetic-retry-policy/2` | 2026-01-02 11:00 | 检查输出：3 passed; 0 failed。 |
| `codex/synthetic-retry-policy/3` | 2026-01-02 12:00 | 下一步：加取消场景。 |
| `codex/synthetic-color-theme/1` | 2026-01-03 10:00 | 不相关的更新任务，已完成。 |

恢复说明应引用当前三条记录。color-theme 和旧的上限 5 方案不是要继续的线程。

这条路径由 `tests/mcp_session_recovery.rs` 覆盖，使用 `SIVTR_HOME` 下的合成 WorkSet。fixture 不能证明当前检出上那些虚构命令真的跑过。

见 [MCP 参考](/zh-cn/reference/cli/#mcp) 和 [Agent 交接](/zh-cn/playbooks/agent-handoff/)。
