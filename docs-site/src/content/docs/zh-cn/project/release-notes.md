---
title: 发布说明
description: sivtr 发布说明和 changelog 约定。
---

这里作为公开发布说明入口。项目开始持续打 tag 后，详细版本条目应保存在 `CHANGELOG.md`。

## 当前状态

`sivtr` 处于早期 `0.1.x` 开发阶段。当前文档覆盖的功能包括：

- pipe 和 run 捕获模式；
- Vim 风格 TUI 浏览和选择；
- 结构化 shell 会话日志；
- 命令块复制、diff 和选择器工作流；
- Codex 会话复制工作流；
- SQLite 历史搜索；
- TOML 配置；
- Windows 全局 Codex 选择器热键。

## 建议 changelog 格式

```markdown
## [Unreleased]

### Added

### Changed

### Fixed

### Removed
```

每个发布版本写明日期和面向用户的变化：

```markdown
## [0.1.1] - 2026-04-28

### Added

- Added `sivtr copy codex out --pick`.

### Fixed

- Fixed prompt rendering for multiline prompts.
```

发布说明应写给用户看，不要直接堆原始 commit message。
