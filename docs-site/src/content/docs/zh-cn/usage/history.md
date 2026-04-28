---
title: 历史
description: 列出、搜索和显示已保存的输出历史。
---

`sivtr` 会把捕获输出保存在本地 SQLite 历史数据库中，并支持 FTS5 全文搜索。历史命令以读取为主：列出最近条目、按关键词搜索，以及显示完整条目。

## 列出最近条目

```bash
sivtr history
sivtr history list
sivtr history list --limit 50
```

输出包含条目 id、时间戳、命令和内容预览。

## 搜索

```bash
sivtr history search "panic"
sivtr history search "failed assertion" --limit 10
```

搜索使用历史全文索引。拿到结果 id 后可用 `history show` 查看。

## 显示条目

```bash
sivtr history show 42
```

详情视图会先打印元数据，再打印保存的内容：

- id；
- 时间戳；
- 命令；
- 来源；
- 主机；
- 内容。

## 保留策略

历史保留由配置控制：

```toml
[history]
auto_save = true
max_entries = 0
```

`max_entries = 0` 表示不限制。
