---
title: 文档维护
description: Rust 工具变化时，如何保持文档站准确。
---

这个站点应尽量贴近代码。风险最高的页面是命令参考、键位和配置参考，因为它们直接镜像 Rust 定义。

## 事实来源

| 文档页 | 代码来源 |
| --- | --- |
| CLI 参考 | `src/cli.rs` |
| 键位 | `src/tui/event.rs`、`src/app.rs`、`src/commands/copy.rs` |
| 配置文件 | `crates/sivtr-core/src/config/mod.rs` |
| 会话模型 | `crates/sivtr-core/src/session/entry.rs` |
| 架构 | workspace 布局和 `crates/sivtr-core/src/lib.rs` |

## 更新清单

修改 CLI 时：

1. 更新 `src/cli.rs` 中的 clap help。
2. 更新 [CLI 参考](/zh-cn/reference/cli/)。
3. 在使用页中添加或更新示例。
4. 运行文档构建。

修改 TUI 键位时：

1. 更新[键位](/zh-cn/reference/keybindings/)。
2. 更新提到这些键位的任务页。
3. 检查快速开始仍然有效。

修改配置时：

1. 更新[配置文件](/zh-cn/reference/config-file/)。
2. 更新[配置](/zh-cn/usage/configuration/)。
3. 确认 `sivtr config show` 输出仍与文档匹配。

## 本地构建

在 `docs-site/` 下：

```bash
npm install
npm run build
npm run dev
```

## 发布

生成的网站是静态站，任何静态托管都可以使用：

- Cloudflare Pages；
- GitHub Pages；
- Vercel；
- Netlify；
- 普通静态文件服务器。

构建输出目录是：

```text
docs-site/dist/
```
