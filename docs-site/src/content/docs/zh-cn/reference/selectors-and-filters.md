---
title: Selector 和 Filter
description: recent-item selector、session、regex filter、line filter 和 ref 的共享语法。
---

多个 `sivtr` 命令共享一套小语法，用于选择和裁剪文本。本页把这些规则集中说明。

## Recency selector

Selector 用来选择最近的命令块或 AI 单元。

| Selector | 含义 |
| --- | --- |
| 省略 | 最新匹配项 |
| `1` | 最新匹配项 |
| `2` | 第二新的匹配项 |
| `2..4` | 一段最近匹配项 |

示例：

```bash
sivtr copy out
sivtr copy out 2
sivtr copy in 2..4
sivtr copy claude out 2
sivtr copy codex 2..4
```

Selector 从新到旧计数，因为最常复用的目标通常刚刚发生。

## Diff selector

`diff` 使用同样的 recency 编号，但左右两边都必须解析成单个命令块：

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
```

`2..4` 这种范围 selector 不能作为 diff 的一边。

## AI session 选择

Agent provider 命令可以把 session 选择和 item selector 分开：

```bash
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy claude out --session 3
```

`--session N` 选择 picker 流程中同一排序下第 N 新的可选 session。`--session ID` 匹配 session id 或 id 前缀。

## Regex filter

`--regex <PATTERN>` 会在选中文本组装后只保留匹配行：

```bash
sivtr copy out --regex panic
sivtr copy claude tool --regex "error|failed"
```

当 shell 可能解释正则字符时，请加引号。

## Line filter

`--lines <SPEC>` 会在选中文本组装后保留 1-based 行范围：

```bash
sivtr copy out --lines 10:20
sivtr copy out --lines 1,3,8:12
sivtr copy codex all --lines 1:40
```

常见形式：

| Spec | 含义 |
| --- | --- |
| `5` | 第 5 行 |
| `1:5` | 第 1 到第 5 行 |
| `10:20` | 第 10 到第 20 行 |
| `1,3,8:12` | 第 1、3、8 到 12 行 |

同时设置 `--regex` 和 `--lines` 时，`--regex` 先运行，`--lines` 再作用于过滤后的结果。

## WorkSet 引用

WorkSet 命令（`search`、`filter`、`nav`、`zoom`、`show`、`work records` 和 `work parts`）共享 source 形式：

| Source | 含义 |
| --- | --- |
| `@last` | 最近一次 WorkSet 命令产生的 WorkSet。 |
| `@name` | 通过 `--save name` 或 `sivtr var set name` 保存的命名 WorkSet。 |
| `@name[1,3..5]` | 已保存 WorkSet 的 1-based 切片。离散 selector 保留请求顺序。 |
| `@` | 从 stdin 读取 WorkSet JSON。不要把 `--refs` 文本管给 `@`。 |

WorkSet 包含 materialized `records` 和 active `anchors`。`filter` 缩小 anchors，`nav` 移动 anchors，`var` 记住 anchors，`show` 渲染 anchors。

## Anchor motion

`sivtr nav <source> <motion>` 使用一套小的确定性 motion 语法。它不会默认展开 child。

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
sivtr nav @hit '<' --refs
sivtr nav @hit '<+1>1' --refs
sivtr nav @hit '<[-2..+2]' --refs
sivtr nav @hit '~' --refs
```

## Prompt 重写

能复制输入的命令块模式可以重写 prompt：

```bash
sivtr copy in --prompt ":"
sivtr copy --prompt ">"
```

如果 prompt 结尾没有空白，`sivtr` 会在命令前插入一个空格。

## ANSI 保留

当 source 有保存过的 ANSI 内容时，用 `--ansi` 复制 ANSI-decorated text：

```bash
sivtr copy out --ansi
```

默认仍是纯文本，因为它更适合搜索、issue 报告和 AI prompt。

## Workspace ref

`search --format refs` 和 `search --format workset` 会输出 `show` 可以打印的 ref：

```text
[origin:]source/session[/dialogue[/line]]
```

本地示例：

```bash
sivtr show claude/<session>
sivtr show claude/<session>/<dialogue>
sivtr show claude/<session>/<dialogue>/<line>
sivtr show terminal/current/<block>
sivtr show terminal/current/<block>/<line>
```

带 origin 的示例（`origin:body`）：

```bash
sivtr show desk:terminal/session_42/3
sivtr show desk:agent/<session>/3/o/1
sivtr show docs:codex/4
sivtr s desk:terminal --status failure --latest 5 --refs
```

origin 来自：

- 用 `sivtr remote add <name> <invite>` 创建的远端名；
- `sivtr ws list` 列出的本机 workspace 目录名。

Dialogue 和 line 索引都是 1-based。
