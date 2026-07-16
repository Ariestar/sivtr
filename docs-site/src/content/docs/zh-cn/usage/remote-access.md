---
title: 远程访问
description: 以只读方式分享 workspace，并用 remote 名挂接另一台设备的记忆（类似 git remote）。
---

跨设备记忆让两台运行 `sivtr` 的机器像读本地 source 一样读取彼此的 workspace session。分享是显式的、只读的，并且默认脱敏。

如果想先看协作场景，见 [远程协作记忆](/zh-cn/playbooks/remote-collaboration-memory/)。本页是功能指南。

## 模型

| 部件 | 含义 |
| --- | --- |
| Device daemon | 每台机器一个。由 `sivtr serve` 启动；share/remote 需要时会自动拉起。 |
| Share | 被显式暴露的本机 workspace（远端「仓库」）。 |
| Pass | 单次使用、短 TTL 的通行证。stdout 打印 bare key（`share pass`）。 |
| Grant | peer 兑换 pass 后获得的权限。 |
| Remote | 当前 workspace 内给 peer+share 起的本地名，成为 `name:path` 左侧（类似 `git remote`）。 |

ref 统一为一种形式：

```text
codex/4                 # 当前本地 workspace
docs:codex/4            # 本机另一个 workspace 名
desk:terminal/...       # remote add 得到的远端名
```

用 `sivtr ws list` 查看本机 workspace 标签。未登记的 scope 会报错。

## 所有者设置

在拥有 workspace 的机器上：

```bash
sivtr share                   # 选择 workspace（Enter = 当前）；只创建 share
sivtr share pass <name>       # 签发单次 pass（stdout = bare key）
```

非交互：

```bash
sivtr share add --name alice-desk
sivtr share pass alice-desk --expires 10m
```

常用所有者命令：

```bash
sivtr share list
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
sivtr share disable alice-desk
sivtr share remove alice-desk
sivtr serve status
```

## 对端设置

在要挂 remote 的 git workspace 里：

```bash
sivtr remote add desk <pass>
sivtr remote test desk
sivtr remote list
```

常用对端命令：

```bash
sivtr remote rename desk bob-desk
sivtr remote remove desk          # 只删本地名；grant 仍在，直到 owner revoke
sivtr peer list
sivtr peer forget <peer>
```

## 使用远端记忆

remote 与本地 source 使用同一套 WorkSet 表面：

```bash
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr s desk:agent -m "panic|failed|decision" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr zoom desk:agent/<session>/3 -C 2 --save remote_ctx --refs
sivtr filter @remote_hits -m "cargo test" --save remote_tests --refs
sivtr nav @remote_tests[1] '<[-1..+1]' --refs
sivtr copy ref desk:terminal/session_42/3/o/1 --print
```

## 安全默认

- 未运行 `sivtr share` / `share add` 前，什么都不会被分享。
- 访问只读。peer 不能写 session，也不能在 owner 上跑命令。
- 默认开启脱敏（`--no-redact` 可关）。
- Pass 单次、短时（默认 `10m`）。
- daemon 之间为加密 iroh 传输。
- 本地优先：未知 scope 直接失败，不会静默扫网。

## Daemon 与数据

```bash
sivtr serve start
sivtr serve status
sivtr serve logs
sivtr serve stop
```

状态在 `data_dir()`（`SIVTR_DATA_DIR` 覆盖，否则平台 config 下的 `sivtr`）：

| 文件 | 用途 |
| --- | --- |
| `identity.key` | 稳定设备身份 |
| `remote-state.db` | peers / shares / grants / passes / remotes |
| `daemon.json` / `daemon.lock` / `daemon.log` | 运行控制与日志 |

见 [数据位置](/zh-cn/reference/data-locations/) 与 [本地优先与隐私](/zh-cn/explanation/local-first-privacy/)。

## 命令表

| 命令 | 用途 |
| --- | --- |
| `sivtr share` | 交互式 share（不出 pass） |
| `sivtr share add\|list\|pass\|grants\|revoke...` | 管理 share |
| `sivtr remote add\|list\|remove\|rename\|test` | 管理当前 workspace 的 remote |
| `sivtr peer list\|forget` | 管理已知 peer |
| `sivtr serve ...` | 管理设备 daemon |
| `sivtr ws list` | 列出本机 workspace 标签 |

精确语法：[CLI 参考](/zh-cn/reference/cli/)。
