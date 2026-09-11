---
title: MCP 跨会话任务恢复
description: 通过真实 MCP 调用、已保存的选择和精确证据引用，恢复一个合成任务。
---

## 场景

Agent 要在新会话中继续 `retry-policy` 任务。最新记录属于另一个任务，而一条更早的 `retry-policy` 记录报告了失败的检查和不同的重试方案。只取最新记录会得到错误的交接内容。

本例使用固定且完全合成的 [WorkSet fixture](https://github.com/Ariestar/sivtr/blob/main/tests/fixtures/mcp-session-recovery.json)，包含目标任务的决策、验证记录和下一步，以及旧记录和更新的不相关记录。其中描述的工作均为虚构，不含私人对话或真实验证结果。

| 记录 | 时间（UTC） | 证据 |
| --- | --- | --- |
| `codex/synthetic-retry-policy-old/1` | 2026 年 1 月 1 日 10:00 | 旧的失败验证；提出重试上限为 5。 |
| `codex/synthetic-retry-policy/1` | 2026 年 1 月 2 日 10:00 | 决策：重试上限为 2，保留指数退避。 |
| `codex/synthetic-retry-policy/2` | 2026 年 1 月 2 日 11:00 | 合成测试输出：3 passed; 0 failed。 |
| `codex/synthetic-retry-policy/3` | 2026 年 1 月 2 日 12:00 | 下一步：添加取消场景。 |
| `codex/synthetic-color-theme/1` | 2026 年 1 月 3 日 10:00 | 不相关的 color-theme 任务标记为完成。 |

## 运行示例

在已满足[开发环境要求](https://github.com/Ariestar/sivtr/blob/main/CONTRIBUTING.md)的源码检出中运行：

```bash
cargo test --locked --test mcp_session_recovery -- --nocapture
```

[集成检查](https://github.com/Ariestar/sivtr/blob/main/tests/mcp_session_recovery.rs)启动编译后的 `sivtr mcp serve --idle-exit 0` 进程，通过 stdio 交换 JSON-RPC 消息。它创建临时数据目录，将包含完整记录的 fixture 保存为 `@fixture`，只查询这个 WorkSet。它为 server 设置 `SIVTR_DATA_DIR`，将 fixture 和保存的选择放在 `SIVTR_DATA_DIR/sets` 下；未设置这个覆盖变量时，原有的平台存储位置不变。它不发现或导入原生 provider 会话。若存在正常的 sivtr 配置，server 仍会读取，因此配置必须有效。

检查核验返回的 ref、来源字段、选中的记录和展开内容，还会在使用已保存选择前重启 server，确认恢复过程不依赖同一个存活的 MCP 进程。

## 搜索并缩小证据范围

完成 MCP 初始化握手和 `tools/list` 后，用任务特定的内容过滤条件调用 `sivtr_search`：

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "sivtr_search",
    "arguments": {
      "source": "@fixture",
      "query": "retry-policy",
      "match_regex": "retry-policy",
      "limit": 10,
      "save": "recovery",
      "detail": "timeline"
    }
  }
}
```

`query` 对结果排序，`match_regex` 限定匹配范围。搜索返回四个 anchor，每个 timeline 条目都有 `source: "codex"`。它排除了更新的不相关记录，但仍会匹配旧记录。将结果保存为 `@recovery`，再缩小到 fixture 中的当前工作时段：

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "sivtr_filter",
    "arguments": {
      "source": "@recovery",
      "since": "2026-01-02T00:00:00Z",
      "save": "current",
      "detail": "workset"
    }
  }
}
```

日期属于这个固定场景。实际工作中，应根据用户请求与已有证据确定目标任务和相关时间范围；时间戳更新本身不能证明一条决策替代了另一条。

MCP 将 JSON 放在 `result.content` 的 text block 中。解析该 block 的 `text`，不要假定存在 `structuredContent` 字段。搜索返回 `anchors`，timeline 模式还返回包含 `ref`、`source`、`time`、`snippet` 的 `items`。这里的过滤调用指定了 `detail: "workset"`，所以结果包含 `anchors` 和带有完整记录的 `workset`。

解码后的过滤结果包含以下字段。这里只省略了其中的 `workset` 字段；测试会检查其完整记录支撑的正是这些 anchor：

```json
{
  "count": 3,
  "anchors": [
    "codex/synthetic-retry-policy/3",
    "codex/synthetic-retry-policy/2",
    "codex/synthetic-retry-policy/1"
  ],
  "saved_as": "current"
}
```

## 从已保存的选择恢复

`sivtr_search` 和 `sivtr_filter` 会更新 `@last`。检查在保存 `@current` 后故意搜索不相关任务，再停止并重新启动 MCP server。新会话用 `sivtr_show` 展开 `@current`，不假定 `@last` 仍然指向目标任务。再次用 `save: "current"` 保存其他结果，会覆盖这个命名选择。

根据过滤结果返回的 `anchors` 选择证据。`@current[1]` 等 WorkSet selector 从 1 开始计数，位置属于这个已保存的选择。使用文本前，逐项比较展开结果的 `contents[].ref` 与相应的返回 anchor。

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "sivtr_show",
    "arguments": {
      "source": "@current[3]",
      "mode": "full"
    }
  }
}
```

下面完整展示解码后的 `sivtr_show` 结果，省略了 JSON-RPC 外层和 text block 包装：

```json
{
  "count": 1,
  "anchors": ["codex/synthetic-retry-policy/1"],
  "contents": [
    {
      "ref": "codex/synthetic-retry-policy/1",
      "content": "SYNTHETIC retry-policy decision: cap retries at 2; keep exponential backoff."
    }
  ]
}
```

WorkRef 标识证据位置。已保存的 WorkSet 保留选中的 anchor 和相应完整记录，只要该集合仍然可用，就可以跨 server 重启使用。MCP 直接接收这些 handle；CLI 的 stdin `@` 管道不是 MCP 传输方式。检查还会展开 `@current[2]` 取得验证记录、展开 `@current[1]` 取得下一步。

## 写出恢复说明

展开三个选中记录后，可以写出这样的有据可查的恢复说明：

- **原始决策：** 1 月 2 日决定重试上限为 2，保留指数退避。来源：`codex/synthetic-retry-policy/1`。
- **原始验证：** 合成的 `cargo test retry_policy --locked` 输出记录了“3 passed; 0 failed”。来源：`codex/synthetic-retry-policy/2`。这是已保存的 fixture 内容，不是刚对当前检出执行的测试。
- **原始下一步：** 在认定任务完成前添加取消场景。来源：`codex/synthetic-retry-policy/3`。
- **Agent 推断：** 从取消场景覆盖继续，检查当前文件并运行相关检查。不相关的 color-theme 记录和历史通过输出，都不足以确认这个任务已经完成。

自动检查验证了这组 fixture 的 stdio 请求链路、选择持久化、排除不相关及陈旧记录、证据展开。它不评价原生 provider 导入、Agent 推理质量、真人时间收益或检索泛化能力。fixture 中报告的验证是合成内容；此处实际执行的是集成检查本身。

server 配置见 [MCP 参考](/zh-cn/reference/cli/#mcp)，更通用的交接流程见 [Agent 交接](/zh-cn/playbooks/agent-handoff/)。
