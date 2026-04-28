---
title: CLI 参考
description: 命令语法、子命令、选项、选择器和示例。
---

本页记录公开 CLI 表面，应与 `src/cli.rs` 保持一致。

## 顶层命令

```bash
sivtr [COMMAND]
```

如果没有提供命令，`sivtr` 会读取 stdin，与管道模式一致。

## run

```bash
sivtr run <COMMAND> [ARGS...]
```

运行命令，捕获合并输出，报告退出状态，然后打开捕获的输出。

示例：

```bash
sivtr run cargo test
sivtr run git status --short
```

## pipe

```bash
sivtr pipe
```

读取 stdin 并打开。通常直接管道到 `sivtr` 等价：

```bash
cargo build 2>&1 | sivtr
```

## import

```bash
sivtr import
```

打开当前结构化会话日志。需要 shell 集成。

## init

```bash
sivtr init <SHELL>
```

支持的 shell 名称：

- `powershell`
- `pwsh`
- `bash`
- `zsh`
- `nushell`
- `nu`

## copy

```bash
sivtr copy [MODE] [SELECTOR] [OPTIONS]
```

模式：

| 模式 | 含义 |
| --- | --- |
| 不写模式 | 复制输入加输出 |
| `in` | 复制输入 |
| `out` | 复制输出 |
| `cmd` | 复制裸命令 |
| `codex` | 复制 Codex 会话内容 |

别名：

| 别名 | 展开为 |
| --- | --- |
| `c` | `copy` |
| `ci` | `copy in` |
| `co` | `copy out` |
| `cc` | `copy cmd` |

常用选项：

| 选项 | 含义 |
| --- | --- |
| `--ansi` | 可用时复制带 ANSI 装饰的文本 |
| `--pick` | 打开交互式选择器 |
| `--print` | 复制后打印文本 |
| `--regex <PATTERN>` | 只保留匹配正则的行 |
| `--lines <SPEC>` | 只保留 1-based 行范围 |

支持输入的模式还支持：

| 选项 | 含义 |
| --- | --- |
| `--prompt <TEXT>` | 重写复制输入里的 prompt |

示例：

```bash
sivtr copy
sivtr copy 3 --print
sivtr copy --prompt ":"
sivtr copy in 2..4
sivtr copy out --pick --regex panic
sivtr copy cmd --pick
```

## copy codex

```bash
sivtr copy codex [MODE] [SELECTOR] [OPTIONS]
```

模式：

| 模式 | 含义 |
| --- | --- |
| 不写模式 | 最近已完成的用户消息加助手回复 |
| `out` | 最近助手回复 |
| `in` | 最近用户消息 |
| `tool` | 最近工具输出 |
| `all` | 整个解析后的会话 |

示例：

```bash
sivtr copy codex
sivtr copy codex 2
sivtr copy codex 2..4
sivtr copy codex out --print
sivtr copy codex out --pick
sivtr copy codex tool --regex error
sivtr copy codex all --lines 1:20
```

## diff

```bash
sivtr diff <LEFT> <RIGHT> [OPTIONS]
```

比较当前会话里的两个最近命令块。每个选择器必须解析为单个块。

内容模式：

| 选项 | 含义 |
| --- | --- |
| `--output` | 比较输出文本，默认值 |
| `--block` | 比较输入加输出 |
| `--input` | 比较带 prompt 的输入 |
| `--cmd` | 比较裸命令文本 |

视图选项：

| 选项 | 含义 |
| --- | --- |
| `--side-by-side` | 显示双栏文本视图 |

示例：

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
sivtr diff 2 1 --side-by-side
```

## history

```bash
sivtr history [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `list [-l, --limit <N>]` | 列出最近条目 |
| `search <KEYWORD> [-l, --limit <N>]` | 搜索历史 |
| `show <ID>` | 显示指定条目 |

如果没有提供 history 子命令，`sivtr` 会列出最近 20 条。

## config

```bash
sivtr config [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `show` | 显示配置路径和内容 |
| `init` | 创建默认配置 |
| `edit` | 在编辑器中打开配置 |

如果没有提供 config 子命令，默认使用 `show`。

## hotkey

```bash
sivtr hotkey [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `start [--chord <CHORD>]` | 启动 Windows 热键守护进程 |
| `status` | 显示守护进程状态 |
| `stop` | 停止守护进程 |

如果没有提供 hotkey 子命令，默认使用 `status`。

## clear

```bash
sivtr clear [--all]
```

清理会话日志。`--all` 会清理所有记录的会话日志和状态文件。

## 选择器语法

| 选择器 | 含义 |
| --- | --- |
| 省略 | `1` |
| `1` | 最新匹配项 |
| `2` | 第二新的匹配项 |
| `2..4` | 最近范围 |

命令块复制、Codex 复制和 diff 在适用时共享选择器语义。
