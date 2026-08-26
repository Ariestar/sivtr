---
title: 远程访问
description: 以只读方式分享 workspace，并用 remote 名挂接另一台设备的记忆（类似 git remote）。
---

跨设备记忆让两台运行 `sivtr` 的机器像读本地 source 一样读取彼此的 workspace session。分享是显式的、只读的，并且默认脱敏。

如果想先看协作场景，见 [远程协作记忆](/zh-cn/playbooks/remote-collaboration-memory/)。本页是功能指南。

## 先决定用 `publish` 还是 `share`

两者都叫“分享”，但授权边界不同：

| 需求 | 使用 | 结果 |
| --- | --- | --- |
| 给不安装 Sivtr 的人一个浏览器链接，发布已经确定的内容 | [`sivtr publish`](/zh-cn/usage/publish/) | 不可变的加密只读快照；发布者可以离线；链接到期或可撤销 |
| 让另一台 Sivtr 继续搜索一个 workspace 的当前记忆 | `sivtr share` + `sivtr remote` | 只读 workspace mount；需要 daemon、peer 授权；读取的是当前内容 |

`publish preview --pick` 的原子选择只作用于浏览器快照，不会改变 `share` 暴露的 workspace，也不会把 remote/group 内容变成可发布输入。需要按 User、Assistant、Tool、Skill、Thinking 原子挑选时，请先阅读 [发布浏览器只读对话链接](/zh-cn/usage/publish/)；需要跨设备检索时，继续按本页的 share/invite/remote 流程操作。

## 模型

| 部件 | 含义 |
| --- | --- |
| Device daemon | 每台机器一个。由 `sivtr serve` 启动；share/remote 需要时会自动拉起。 |
| Share | 被显式暴露的本机 workspace（远端「仓库」）。 |
| Invite | 单次使用、短 TTL 的邀请 key。stdout 打印 bare key（`share invite`）。 |
| Grant | peer 兑换 invite 后获得的权限。 |
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
sivtr share invite <name>       # 签发单次 invite（stdout = bare key）
```

非交互：

```bash
sivtr share add --name alice-desk
sivtr share invite alice-desk --expires 10m
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
sivtr remote add desk <invite>
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

## 群组（group）

当两台以上设备需要长期共享记忆时，与其每对都做 `share` + `remote add`，不如组成一个**群组**：一组互相共享记忆的设备。每个成员贡献 workspace，成员自动同步。

```bash
# 组主（owner）在 A 机上：
sivtr group create <name>        # 建组并贡献当前 workspace
sivtr group invite <name> --expires 1d --max-uses 10   # 多设备 join 链接（stdout = bare key）

# 成员在 B 机上：
sivtr group join <invite>        # 加入并贡献自己的 workspace
sivtr group list
sivtr group members <name>
sivtr group sync <name>          # 强制从 owner 拉一次成员清单
```

组由 owner 管理：`rename` 改名、`remove <group> <peer>` 踢人，owner 退出（`leave`）会解散整组。成员变更自动广播给每个成员，队友的记忆会直接出现在他们的 peer 名下，无需额外设置：

```bash
sivtr s <peer>:terminal --status failure --latest 5 --refs
sivtr s <peer>:agent -m "decision" --latest 20 --refs
```

群组访问是只读的，默认脱敏密钥，与 share 一致。群组与一次性 share 相互独立：可以只用其中一种，也可以都用。

## 使用远端记忆

remote 与本地 source 使用同一套 WorkSet 表面：

```bash
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr s desk:agent -m "panic|failed|decision" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/p1 --full
sivtr zoom desk:agent/<session>/3 -C 2 --save remote_ctx --refs
sivtr filter @remote_hits -m "cargo test" --save remote_tests --refs
sivtr nav @remote_tests[1] '<[-1..+1]' --refs
sivtr copy desk:terminal/session_42/3 --print
```

## 安全默认

- 未运行 `sivtr share` / `share add` 前，什么都不会被分享。
- 访问只读。peer 不能写 session，也不能在 owner 上跑命令。
- 默认开启脱敏（`--no-redact` 可关）。
- Invite 单次、短时（默认 `10m`）。
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
| `remote-state.db` | peers / shares / grants / invites / remotes |
| `daemon.json` / `daemon.lock` / `daemon.log` | 运行控制与日志 |

见 [数据位置](/zh-cn/reference/data-locations/) 与 [本地优先与隐私](/zh-cn/explanation/local-first-privacy/)。

## 命令表

| 命令 | 用途 |
| --- | --- |
| `sivtr share` | 交互式 share（不出 invite） |
| `sivtr share add\|list\|invite\|grants\|revoke...` | 管理 share |
| `sivtr remote add\|list\|remove\|rename\|test` | 管理当前 workspace 的 remote |
| `sivtr group create\|invite\|join\|list\|members\|remove\|rename\|leave\|sync` | 管理共享记忆群组 |
| `sivtr origin rename` | 改本地 workspace 别名或远端 mount |
| `sivtr peer list\|forget` | 管理已知 peer |
| `sivtr serve ...` | 管理设备 daemon |
| `sivtr ws list` | 列出本机 workspace 标签 |

精确语法：[CLI 参考](/zh-cn/reference/cli/)。
