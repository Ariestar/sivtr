---
title: 修复最近的终端报错
description: Agent 自己找到并修复终端报错。
---

## 演示

<img src="/demo/1.gif" alt="Agent 用 sivtr 记忆修复最近终端失败" class="demo-wide" />

## 场景

你运行了一个命令，失败了，想让 Agent 修。有了 `sivtr`，Agent 可以搜索最近的终端输出、读取相关文件、修复问题，并验证结果。

## 你只需要说

```text
解决终端报错。
```

命令失败后这样说：

## 处理过程

Agent 在最近的 workspace 记忆中搜索失败信号：

```bash
sivtr search terminal --match "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --format json --limit 20
sivtr search terminal --status failure --latest 1 --format json
```

如果搜索片段不够，再展开最近一条命令输出：

```bash
sivtr copy out 1 --print
```

然后它读相关源文件、修复问题、重新运行失败命令，并用验证输出汇报结果。

## 交互示例

```text
用户：解决终端报错。

Agent：我在 sivtr 记忆中找到最近失败：`cargo test` 报 error[E0432]
——未解析的 import `crate::config::legacy`。
我删除了 src/commands/show.rs 中的过期 import，重新运行
`cargo test`，42 个测试全部通过。
```

## 视频演示大纲

1. 运行一个会失败的命令。
2. 对 Agent 说："解决终端报错。"
3. 展示 Agent 用 `sivtr` 搜索失败原因。
4. 展示修复和验证输出。
