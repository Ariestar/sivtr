---
title: 远程协作记忆
description: 通过命名 remote 读取队友终端或 Agent 记忆的场景玩法。
---

## 场景

你在修一个 bug，另一个队友的 Agent 已经调查过了。经过授权后，你想直接搜索他们的 Agent 记忆，查看已经尝试过的方案——不用离开本机，也不用互相粘贴日志。

功能设置、安全默认和命令地图见 [远程访问](/zh-cn/usage/remote-access/)。本页是场景路径。

## 你会说

```text
Alice 的 Agent 在这个 bug 上试过什么？
我继续之前，给我看 Bob 那边的验证输出。
```

## 一次性设置

两端都需要安装 `sivtr`。share / remote 命令需要 daemon 时会自动启动。

在拥有 workspace 的设备上：

```bash
sivtr share                   # 交互选择 workspace（Enter = 当前）；只创建 share
sivtr share invite <name>     # 签发单次 invite（stdout = bare key）
```

在另一台设备的目标 workspace 里：

```bash
sivtr remote add desk <invite>
sivtr remote test desk
```

## 日常工作流

```bash
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr s desk:agent -m "panic|failed|decision" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr zoom desk:agent/<session>/3 -C 2 --save remote_ctx --refs
sivtr show @remote_ctx -f timeline
sivtr filter @remote_hits -m "cargo test" --save remote_tests --refs
```

## 收尾

所有者：

```bash
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
```

对端：

```bash
sivtr remote list
sivtr remote remove desk
```

## 视频演示大纲

两台机器：所有者跑 `sivtr share` 再 `share invite`，消费者把 invite 兑换为 remote `desk`，搜索 `desk:terminal` 找到失败，zoom 命中，再从队友的证据继续修。
