---
title: 复制命令块
description: 复制最近的命令输入、输出、命令、范围和过滤后的行。
---

`sivtr copy` 基于 shell 集成创建的结构化会话日志工作，不需要打开 TUI。

## 基本模式

```bash
sivtr copy
sivtr copy in
sivtr copy out
sivtr copy cmd
```

| 命令 | 复制内容 |
| --- | --- |
| `sivtr copy` | 输入加输出 |
| `sivtr copy in` | 只复制输入，默认包含 prompt |
| `sivtr copy out` | 只复制输出 |
| `sivtr copy cmd` | 只复制裸命令 |

别名：

| 别名 | 完整命令 |
| --- | --- |
| `sivtr c` | `sivtr copy` |
| `sivtr ci` | `sivtr copy in` |
| `sivtr co` | `sivtr copy out` |
| `sivtr cc` | `sivtr copy cmd` |

## 选择最近的块

选择器相对于最新命令块：

```bash
sivtr copy 1
sivtr copy out 2
sivtr copy in 2..4
```

`1` 是最新块，`2` 是上一个块，`2..4` 选择多个最近块。

## 复制后打印

用 `--print` 查看复制了什么：

```bash
sivtr copy out --print
```

文本仍然会复制到剪贴板。

## 保留 ANSI

如果想保留彩色终端序列，使用 `--ansi`：

```bash
sivtr copy out --ansi
```

只有会话条目保存了 ANSI 输出时，这个选项才有效。

## 重写 prompt

复制输入的模式默认保留原始 prompt。可以用 `--prompt` 覆盖：

```bash
sivtr copy in --prompt ":"
sivtr copy --prompt ">"
```

如果 prompt 不以空白结尾，`sivtr` 会在命令前插入一个空格。

## 过滤复制文本

过滤在选中的块合并后执行。

```bash
sivtr copy out --regex panic
sivtr copy out --lines 10:20
sivtr copy out --lines 1,3,8:12
```

如果两个过滤器都设置了，`--regex` 先运行，`--lines` 再作用于过滤结果。

## 交互式选择器

打开交互式选择器：

```bash
sivtr copy --pick
sivtr copy out --pick
sivtr copy cmd --pick
```

选择器按键：

| 按键 | 动作 |
| --- | --- |
| `j` / `k` | 移动 |
| `Space` | 切换当前条目 |
| `v` | 标记范围锚点 |
| `a` | 全选/全不选 |
| `p` | 切换预览 |
| `t` | 打开 Vim 风格完整视图 |
| `Enter` | 确认 |
| `Esc` | 取消 |

Vim 风格完整视图支持 `[[` 和 `]]` 跳转块，Codex 视图中 `T` 可在有替代完整视图时切换工具内容，`myy` / `myi` / `myo` / `myc` 可复制，`mvv` / `mvi` / `mvo` 可选择。
