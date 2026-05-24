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

`search --format json` 会输出 `show` 可以打印的 ref：

```text
source/session[/dialogue[/line]]
```

示例：

```bash
sivtr show claude/<session>
sivtr show claude/<session>/<dialogue>
sivtr show claude/<session>/<dialogue>/<line>
sivtr show terminal/current/<block>
sivtr show terminal/current/<block>/<line>
```

Dialogue 和 line 索引都是 1-based。
