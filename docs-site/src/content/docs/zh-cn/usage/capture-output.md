---
title: 捕获终端输出
description: 使用 pipe mode、run mode 和 shell session integration。
---

捕获是把终端输出变成可复用文本的第一步。选择与你的需求匹配的最轻路径即可。

## 选择捕获方式

| 场景 | 推荐命令 | 保留命令元数据？ |
| --- | --- | --- |
| 查看已有命令管道的输出 | `command 2>&1 \| sivtr` | 否 |
| 让 `sivtr` 执行一次命令 | `sivtr run command` | 部分保留本次运行信息 |
| 在活动 shell 中记录每条命令 | `sivtr init <shell>` | 是，含退出码与目录 |
| 浏览当前 workspace 记录的工作 | `sivtr` | 是，需要 shell 集成 |
| 复制最近命令块 | `sivtr copy out` | 是，需要 shell 集成 |
| 搜索捕获的终端和 AI 工作 | `sivtr search "query"` | 是，来自统一 archive |

## Pipe mode

Pipe mode 会把 stdin 写入统一 archive，并用外部编辑器打开结果。

```bash
ls -la | sivtr
cargo build 2>&1 | sivtr
rg "TODO" . | sivtr
```

适合在这些情况下使用：

- 命令已经存在于你的 shell 历史里；
- 你希望保持普通 shell 的管道和重定向行为；
- 不需要 `sivtr` 知道原始命令是什么。

如果重要输出写到了 stderr，把 stderr 重定向到 stdout：

```bash
cargo test 2>&1 | sivtr
```

## Run mode

Run mode 由 `sivtr` 执行命令：

```bash
sivtr run cargo test
sivtr run git status --short
```

适合在这些情况下使用：

- 你希望 `sivtr` 执行并捕获单个命令；
- 你希望编辑器打开前先报告退出状态并把输出存入统一 archive；
- 你不想手动处理 shell 重定向。

Run mode 会合并捕获 stdout 和 stderr，并把结果存入统一 archive。如果命令没有输出，`sivtr` 会提示没有捕获内容后退出。

## Shell session browsing

Shell 集成会持续记录结构化命令条目。安装后打开 workspace 浏览器：

```bash
sivtr
```

当你已经正常工作了一段时间，之后想浏览累积的终端工作时，这很有用。

安装或升级 shell 集成即可使用终端捕获：

```bash
sivtr init all
# 或只安装一个 shell：sivtr init bash / zsh / nushell / powershell
```

`sivtr setup` 会执行同样的安装步骤。安装后重启 shell，后续命令会自动通过 PTY 捕获，无需额外启用。升级后重新运行 `sivtr init <shell>` 会原位替换旧的 sivtr hook，保留周围的用户配置，不会叠加另一套捕获代码。

要暂停捕获，运行 `sivtr config edit`，将 `[pty_proxy]` 下的 `enabled` 设为 `false`，然后重启 shell。`setup` 和 `init` 都会保留这个显式关闭设置。重新开启时将它改回 `true` 并重启 shell。

## 搜索捕获输出

`pipe` 和 `run` 记录为 terminal session，与 shell 和 Agent session 使用同一个 `archive.db`。通过常规 archive 命令搜索或查看：

```bash
sivtr search "panic"
sivtr show terminal/<session>/<record>
```
