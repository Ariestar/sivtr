---
title: Local-first 与隐私
description: sivtr 如何让 Agent memory、终端输出和 transcript 保持在本地用户控制之下。
---

`sivtr` 围绕本地 Agent memory 设计。终端输出、shell session log、history 和 Agent transcript 可能包含密钥、私有代码、凭据、内部 URL 和未完成推理。默认姿态是让这些数据留在原本产生它们的机器上。

## 默认本地

`sivtr` 读写本地文件和数据库：

- shell 集成产生的 shell session log；
- 捕获终端输出的本地 SQLite history；
- provider 自己的 Agent transcript 文件或数据库；
- 平台配置目录下的本地配置。

默认不提供托管 transcript 服务。

## 显式导出

导出是显式用户动作。例如 Codex mirror 需要目标路径：

```bash
sivtr codex export --dest /srv/sivtr/root-codex
```

导出后，普通文件系统权限和你的共享设置决定谁能读取导出的树。

## 显式远程分享

跨设备记忆访问同样是 opt-in。只有你创建 share（`sivtr share` / `share add`）、签发 invite（`share invite`），并且 peer 兑换之后，数据才会离开本机：

```bash
sivtr share                   # 交互式；只创建 share
sivtr share invite alice-desk # 单次 invite（stdout = bare key）
sivtr remote add desk <invite> # peer 在其 workspace 里给 remote 起名
```

远程访问是只读的。数据离开本机前默认脱敏（`--no-redact` 可关闭）。邀请会过期（默认 `10m`）。daemon 之间走加密 iroh。默认仍然本地优先：未登记的 origin 会报错。

完整指南见 [远程访问](/zh-cn/usage/remote-access/)。

## 共享 mirror 应尽量只读

在本机多账号之间共享导出 session 时，建议给消费者只读权限：

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

共享/镜像的 Codex tree 只参与显式 picker 浏览，不会覆盖隐式当前 session 查找。

## 剪贴板是输出边界

Copy 命令会把选中文本放入系统剪贴板：

```bash
sivtr copy out
sivtr copy claude out
```

请把剪贴板内容视为会被桌面环境和剪贴板管理器共享。敏感场景下可用 `--print` 先检查文本。

## History 保留可配置

启用时，捕获的终端输出会保存到 history：

```toml
[history]
auto_save = true
max_entries = 0
```

如果不希望 capture 自动写入，设置 `auto_save = false`。把 `max_entries` 设为正数可以限制保留数量。

## 良好操作习惯

- 除非访问权限可控，否则不要导出包含秘密的目录。
- 把内容粘贴到公开聊天、issue、托管 Agent 或外部 AI 工具前，先检查复制文本。
- 用 line 和 regex filter 只复制必要证据。
- 共享 Codex mirror 与源账号 live config 分开。
- 工具链使用 `--format json` / `--refs` search 输出时，也要记住 JSON content 可能包含敏感文本。
- 协作结束后优先用短寿命 invite，并用 `sivtr share revoke` 撤销 grant。
