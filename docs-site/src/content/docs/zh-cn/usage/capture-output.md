---
title: 捕获输出
description: 使用管道模式、run 模式和会话日志。
---

## 管道模式

管道模式读取 stdin 并打开结果。

```bash
ls -la | sivtr
cargo build 2>&1 | sivtr
rg "TODO" . | sivtr
```

适合在以下场景使用：

- 命令已经在 shell 历史里；
- 你希望保留普通 shell 的管道和重定向行为；
- 不需要 `sivtr` 知道原始命令。

如果重要输出写到了 stderr，请重定向：

```bash
cargo test 2>&1 | sivtr
```

## Run 模式

Run 模式通过 `sivtr` 执行命令：

```bash
sivtr run cargo test
sivtr run git status --short
```

适合在以下场景使用：

- 希望 `sivtr` 直接捕获命令；
- 希望浏览前打印退出状态；
- 不想手动处理 shell 重定向。

Run 模式捕获合并后的输出。如果命令没有产生输出，`sivtr` 会报告没有捕获内容后退出。

## 会话导入

安装 shell 集成后，`sivtr import` 会打开当前会话日志：

```bash
sivtr import
```

当你已经正常工作了一段时间，后来想把累积会话作为一个工作区浏览时，这很有用。

## 选择捕获路径

| 使用场景 | 最佳命令 |
| --- | --- |
| 检查单个命令输出 | `command 2>&1 \| sivtr` |
| 通过工具运行命令 | `sivtr run command` |
| 浏览这个 shell 里记录的一切 | `sivtr import` |
| 不打开 TUI，复制最近命令块 | `sivtr copy out` |
| 搜索已保存捕获 | `sivtr history search "query"` |
