---
title: 会话模型
description: Shell 集成如何记录和复用命令块。
---

Shell 集成把命令块记录为结构化 JSONL 条目。这给 `sivtr` 的 copy、diff、import 和命令块导航提供了可靠数据源。

## 条目形状

每个条目都是一个 `SessionEntry`：

```json
{
  "prompt": "PS C:\\repo> ",
  "command": "cargo test",
  "output": "test result: ok",
  "prompt_ansi": "...",
  "output_ansi": "..."
}
```

当 `prompt_ansi` 和 `output_ansi` 与纯文本相同或不可用时，会被省略。

## 归一化

在构造和加载边界，条目会被归一化：

- CRLF 转换为 LF；
- 去掉末尾换行；
- 从纯 prompt 和 output 中去除 ANSI；
- 当 ANSI 内容与纯文本不同时单独保留。

这让纯文本操作稳定，同时仍然能为 `--ansi` 保留 ANSI 输出。

## 渲染输入

输入部分由 prompt 加 command 渲染而成。

如果 prompt 以换行结尾，命令会放到下一行。否则命令追加到 prompt 的最后一行。

示例：

```text
PS C:\repo> cargo test
```

多行 prompt：

```text
repo on main
> cargo test
```

## 为什么选择器按新近性解释

最常复用的目标就是刚发生的内容。新近性选择器让这个操作很便宜：

```bash
sivtr copy out      # 最新输出
sivtr copy out 2    # 上一个输出
sivtr copy 2..4     # 多个最近块
```

这样用户不需要为临时终端工作记住绝对 id。

## 无效日志

如果会话日志无法解析为结构化条目，`sivtr` 会在追加新条目前重置无效日志。这能保护正常工作流不受损坏文件或旧格式文件影响。

## 旧版本兼容

配置路径解析器会先检查当前 `sivtr` 配置路径。如果没有当前配置，但存在旧的 `sift/config.toml`，则读取旧文件。
