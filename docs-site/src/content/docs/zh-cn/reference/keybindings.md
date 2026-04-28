---
title: 键位
description: TUI、选择器和 Vim 视图键位。
---

## 浏览器

| 按键 | 模式 | 动作 |
| --- | --- | --- |
| `j` / `Down` | Normal, Insert, Visual | 下移 |
| `k` / `Up` | Normal, Insert, Visual | 上移 |
| `h` / `Left` | Normal, Insert, Visual | 左移 |
| `l` / `Right` | Normal, Insert, Visual | 右移 |
| `0` / `Home` | Normal, Insert, Visual | 行首 |
| `^` | Normal, Insert, Visual | 第一个非空字符 |
| `$` / `End` | Normal, Insert, Visual | 行尾 |
| `Ctrl-D` | Normal, Insert, Visual | 下半页 |
| `Ctrl-U` | Normal, Insert, Visual | 上半页 |
| `Ctrl-F` / `PageDown` | Normal, Insert, Visual | 下一页 |
| `Ctrl-B` / `PageUp` | Normal, Insert, Visual | 上一页 |
| `gg` | Normal, Visual | 顶部 |
| `G` | Normal, Visual | 底部 |
| `H` | Normal, Visual | 视口顶部 |
| `M` | Normal, Visual | 视口中部 |
| `L` | Normal, Visual | 视口底部 |
| `/` | Normal, Insert | 搜索 |
| `n` | Normal | 下一个匹配 |
| `N` | Normal | 上一个匹配 |
| `i` | Normal | 插入模式 |
| `v` | Normal, Visual | 字符选择 |
| `V` | Normal, Visual | 行选择 |
| `Ctrl-V` | Normal, Visual | 块选择 |
| `o` | Visual | 交换选择锚点 |
| `y` | Visual | 复制选择 |
| `e` | Normal, Visual | 打开编辑器 |
| `Esc` | Insert, Visual, Search | 取消当前模式 |
| `q` | Normal | 退出 |

## 命令块导航

仅当 buffer 中有解析出的命令块时可用。

| 按键 | 动作 |
| --- | --- |
| `[[` | 上一个命令块 |
| `]]` | 下一个命令块 |
| `myy` | 复制当前命令块 |
| `myi` | 复制当前命令输入 |
| `myo` | 复制当前命令输出 |
| `myc` | 复制当前裸命令 |
| `mvv` | 选择当前命令块 |
| `mvi` | 选择当前命令输入 |
| `mvo` | 选择当前命令输出 |

## 复制选择器

| 按键 | 动作 |
| --- | --- |
| `j` / `Down` | 下移 |
| `k` / `Up` | 上移 |
| `Space` | 切换当前条目 |
| `v` | 标记范围锚点 |
| `a` | 全选/全不选 |
| `p` | 切换预览 |
| `t` | 打开 Vim 风格完整视图 |
| `Enter` | 确认 |
| `Esc` | 取消 |

## Vim 风格完整视图

这个视图从选择器中按 `t` 打开。

| 按键 | 动作 |
| --- | --- |
| `[[` | 上一个块 |
| `]]` | 下一个块 |
| `myy` | 复制块 |
| `myi` | 复制输入 |
| `myo` | 复制输出 |
| `myc` | 复制命令 |
| `mvv` | 选择块 |
| `mvi` | 选择输入 |
| `mvo` | 选择输出 |
| `T` | 可用时切换替代工具视图 |
| `p`, `q`, `Esc` | 返回选择器 |
