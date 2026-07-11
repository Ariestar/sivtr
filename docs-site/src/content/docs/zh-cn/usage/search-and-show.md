---
title: 搜索和展示结果
description: 搜索当前 workspace memory，并打印精确 ref。
---

`sivtr search` 查询捕获到的终端记录和受支持的 AI workspace sessions。`sivtr filter` 缩小已有 WorkSet。`sivtr nav` 在 parent / child / sibling / session 结构中移动 anchors。`sivtr show` 打印 ref 或 WorkSet 背后的内容。

当交互式 picker 太重，而你需要给人类工作流、Agent prompt 或其他工具提供脚本友好的记忆时，把这些命令组合使用。它们也是 skill 最安全的基础能力，因为可以非交互运行，并返回精确 ref 或 WorkSet JSON。

例如，“解决终端报错” skill 可以这样开始：

```bash
sivtr search terminal --status failure --latest 1 --json
```

“最近工作 timeline” skill 可以直接使用 timeline renderer：

```bash
sivtr search agent --since today --sort oldest --format timeline
sivtr search terminal --since today --sort oldest --format timeline
```

## 搜索 target

Search 现在是 target-first：

```bash
sivtr search terminal
sivtr search agent
sivtr search codex
sivtr search claude
sivtr search hermes
sivtr search opencode
sivtr search pi
```

Target 可以继续缩小到 session、record/turn 和 line：

```bash
sivtr search pi/019e5941 --match "cargo test"
sivtr search terminal/session_13104/3/12 --format workset
sivtr search pi/019e5941/3-5,7 --match "cargo test"
sivtr search pi/019e5941/3/5-7,10 --format workset
```

record/turn 和 line segment 都是 1-based，支持 `3`、`3-5`、`3,7` 或 `3-5,7`。`*` 表示 wildcard segment。Search selector 只用于缩小输入范围；search 输出仍然返回具体 ref。

使用 `agent` 搜索所有受支持 AI provider，或使用 provider 名只搜一个 provider。

Target 也可以带 origin 前缀（`origin:body`），用于本机另一个 workspace 名或已挂载的远端别名：

```bash
sivtr search desk:terminal --status failure --latest 5 --refs
sivtr search desk:agent -m "decision|failed" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr show docs:codex/4
```

origin 来自 `sivtr remote add <alias> ...` 或 `sivtr wb list`。功能指南见 [远程访问](/zh-cn/usage/remote-access/)。

## 内容过滤

```bash
sivtr search terminal --match "panic|failed"
sivtr search agent --match "TODO|next step|decision"
sivtr search pi --match "workspace picker" --in title
```

`--match` 是大小写不敏感正则。`--in` 选择搜索字段：

| Field | 搜索范围 |
| --- | --- |
| `content` | 合并后的 record content，默认值 |
| `title` | record / dialogue 标题 |
| `session` | session id / title |
| `input` | 用户输入 / 命令输入 |
| `output` | 助手输出 / 命令输出 |
| `command` | 终端命令文本 |
| `all` | 所有可搜索文本 |

## 时间过滤

```bash
sivtr search agent --since today --format timeline
sivtr search terminal --since yesterday --until today --format md
sivtr search pi --last 2h --format compact
```

时间过滤支持 RFC3339 时间戳、Unix 秒/毫秒、`30m`、`2h`、`7d` 这样的相对时间，以及 `today`、`yesterday`、`tomorrow`、`this morning`、`this afternoon`、`this evening`、`tonight`、`now` 等别名。

## 状态、时长和排序

```bash
sivtr search terminal --status failure --latest 1 --json
sivtr search terminal --exit-code 101 --format timeline
sivtr search terminal --min-duration 500ms --sort duration --format compact
```

常用排序：

- `newest`
- `oldest`
- `duration`
- `duration-asc`
- `exit-code`
- `exit-code-asc`

`--latest <N>` 会先保留最新 N 条匹配记录；`--sort` 再控制最终展示顺序。

## 输出格式

```bash
sivtr search agent --since today --format timeline
sivtr search agent --since today --format compact
sivtr search agent --since today --format md
sivtr search agent --since today --format workset
```

格式只是同一组搜索结果的不同视图，不是“人类格式”和“Agent 格式”的硬切分。按下一步要做什么来选：

| Format | 适合场景 |
| --- | --- |
| `timeline` | 按时间扫读、重建交接、发现 gap。人和 Agent 都容易读。 |
| `compact` | 想要低噪声的 time/source/title 列表。 |
| `md` | 复制进笔记、报告、prompt 或 handoff 草稿。 |
| `workset` | 需要让下一条命令或另一个程序解析 refs 和 materialized records。 |
| `refs` | 逐行 plain refs，适合快速查看或复制。 |

Terminal stdout 默认 `full`；piped stdout 默认 `workset`。`--json` 是 `--format workset` 的便捷别名。当任务是理解、回顾、总结时，Agent 也可以直接读 `timeline`、`compact` 或 `md`。

## 过滤 WorkSet

已有 WorkSet 后，用 `filter` 继续缩小范围，避免重复跑宽泛搜索：

```bash
sivtr search terminal --status failure --latest 20 --save failures --refs
sivtr filter @failures --match "panic|compile" --save focused --refs
sivtr filter @focused --parts --io output --kind tool_output --refs
```

在 shell pipeline 里，`@` 从 stdin 读取 WorkSet JSON：

```bash
sivtr search terminal --json | sivtr filter @ -m error --refs
```

不要把 `--refs` 输出管给 `@`；`@` 需要 WorkSet JSON。

## 导航 anchors

当精确移动路径很重要时，用 `nav`。Motion 是确定性的，不会默认展开 child。

| Motion | 含义 |
| --- | --- |
| `<` | 父级。part/line 到 record；record 到所属 session records。 |
| `>N` | 第 N 个 child，1-based。record 的 children 是 parts。 |
| `+N` | 当前层级向后移动 N 个 sibling。 |
| `-N` | 当前层级向前移动 N 个 sibling。 |
| `[A..B]` | 当前层级 sibling window。 |
| `~` | 所属 session records。 |

示例：

```bash
sivtr nav @focused[1] '<' --refs
sivtr nav @focused[1] '<+1>1' --refs
sivtr nav @focused[1] '<[-2..+2]' --refs
sivtr nav @focused[1] '~' --refs
```

只想围绕命中补 record 上下文时用 `zoom`。

## WorkSet 变量

需要把 WorkSet 作为命名本地记忆保留下来时，用 `var`：

```bash
sivtr var set ctx @last
sivtr var list
sivtr var merge ctx @focused @last[1]
sivtr var drop ctx @noise
sivtr show @ctx --full
```

## 展示 ref

Ref/selector 形状如下：

```text
source/session[/record-or-turn[/line]]
source/session/record/<i|o>/<part>
```

具体 ref 指向单个 record、单行或单个 part。作为命令输入时，record/turn 和 line segment 也可以是 `3-5,7` 这样的 selector；输出 ref 仍然是具体锚点。Part ref 使用 `i`（输入）或 `o`（输出）加上 1-based part 索引。

打印一个 record 或 turn：

```bash
sivtr show pi/<session>/<turn>
sivtr show terminal/<session>/<record>
```

打印某条 1-based line：

```bash
sivtr show claude/<session>/<turn>/<line>
sivtr show terminal/<session>/<record>/<line>
```

打印特定的 input 或 output part：

```bash
sivtr show codex/<session>/<turn>/o/1
sivtr show terminal/<session>/<record>/i/2
```

用 selector 语法打印多个 record 或 line：

```bash
sivtr show pi/<session>/3-5,7
sivtr show pi/<session>/3/5-7,10
```

机器可读 WorkSet 输出：

```bash
sivtr show @ctx --json
```

## 实用循环

1. 先用足够窄的搜索拿证据：

   ```bash
   sivtr search terminal --status failure --latest 1 --refs
   sivtr search agent --match "current task|failed|TODO" --since today --format timeline
   ```

2. 保存并缩小可复用结果集：

   ```bash
   sivtr search agent --match "decision|TODO" --latest 20 --save hits --refs
   sivtr filter @hits --match "workspace|nav|filter" --save focused --refs
   ```

3. 需要时移动或扩展 anchors：

   ```bash
   sivtr nav @focused[1] '<[-1..+1]' --refs
   sivtr zoom @focused[1] -C 2 --save ctx --refs
   ```

4. 打印精确内容：

   ```bash
   sivtr show @ctx --full
   sivtr show <source/session/record-or-turn>
   ```

5. 需要紧凑引用、脚本输入或后续 Agent 的上下文句柄时，再使用精确 part/line ref。
