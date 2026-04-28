---
title: 快速开始
description: 捕获输出、搜索、选择文本，并复制命令块。
---

这篇快速开始覆盖主流程：捕获终端输出、浏览输出、复制选中范围，然后复用最近的命令块。

## 1. 把输出管道到浏览器

```bash
cargo test 2>&1 | sivtr
```

TUI 会打开合并后的输出。常用按键：

- `j` / `k` 逐行移动；
- `Ctrl-D` / `Ctrl-U` 半页移动；
- `gg` / `G` 跳到顶部或底部；
- `/panic` 搜索；
- `n` / `N` 跳到下一个或上一个匹配；
- `q` 退出。

## 2. 选择并复制

在浏览器里：

1. 移到文本起点。
2. 按 `V` 开始按行选择。
3. 移到范围终点。
4. 按 `y`。

选中文本会复制到系统剪贴板。

矩形选择用 `Ctrl-V` 代替 `V`。

## 3. 包装一个命令

当你希望 `sivtr` 执行命令并捕获 stdout/stderr 时，使用 `sivtr run`：

```bash
sivtr run cargo test
sivtr run python scripts/check.py
```

`sivtr` 会打印进程退出状态，然后打开捕获的输出。

## 4. 启用命令块复制

先安装一次 shell 集成：

```bash
sivtr init powershell
```

重启 shell，运行几个命令，然后复制最近的块：

```bash
sivtr copy
sivtr copy out
sivtr copy cmd 2
sivtr copy 2..4 --print
```

选择器按“最新优先”解释。`1` 是最新命令块，`2` 是上一个，`2..4` 是最近的一段范围。

## 5. 复制 Codex 输出

在有 Codex 会话的项目目录中：

```bash
sivtr copy codex out
sivtr copy codex in
sivtr copy codex tool --regex error
sivtr copy codex all --lines 1:40
```

`sivtr` 会找到工作目录匹配当前项目的最新 Codex 会话。
