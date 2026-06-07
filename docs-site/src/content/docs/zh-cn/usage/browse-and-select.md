---
title: 浏览和选择
description: 导航浏览器和 workspace picker，搜索输出，选择文本，并交给编辑器。
---

`sivtr` 有两个交互界面：

- **browser**：用于单个捕获输出缓冲区；
- **workspace picker**：用于命令块和 Agent session。

## Browser 导航

内置 browser 是只读的、Vim 风格的界面，用来扫描大型终端输出并提取有用片段。

| 按键 | 动作 |
| --- | --- |
| `j` / `Down` | 下移 |
| `k` / `Up` | 上移 |
| `h` / `Left` | 左移 |
| `l` / `Right` | 右移 |
| `0` / `Home` | 行首 |
| `^` | 第一个非空白列 |
| `$` / `End` | 行尾 |
| `Ctrl-D` | 下翻半页 |
| `Ctrl-U` | 上翻半页 |
| `Ctrl-F` / `PageDown` | 下翻页 |
| `Ctrl-B` / `PageUp` | 上翻页 |
| `gg` | 顶部 |
| `G` | 底部 |
| `H` / `M` / `L` | 视图顶部/中间/底部 |

## Browser 搜索

按 `/`，输入模式，然后按 `Enter`。

| 按键 | 动作 |
| --- | --- |
| `/` | 开始搜索 |
| `Enter` | 执行搜索 |
| `Esc` | 取消搜索输入 |
| `n` | 下一个匹配 |
| `N` | 上一个匹配 |

搜索会跳转到匹配行，并在状态栏显示匹配数量。

## Browser 选择

| 按键 | 动作 |
| --- | --- |
| `v` | 字符选择 |
| `V` | 行选择 |
| `Ctrl-V` | 块选择 |
| `o` | 交换选择锚点 |
| `y` | 复制选择到剪贴板 |
| `Esc` | 取消选择 |

也支持鼠标选择。左键拖动开始选择；按住 `Ctrl` 拖动为块模式。

## Browser 中的命令块快捷键

浏览结构化 session log 时，`sivtr` 可以跳转、复制或选择当前命令块。

| 按键 | 动作 |
| --- | --- |
| `[[` | 上一个命令块 |
| `]]` | 下一个命令块 |
| `myy` | 复制当前命令块 |
| `myi` | 复制当前命令输入 |
| `myo` | 复制当前命令输出 |
| `myc` | 复制当前裸命令 |
| `mvv` | 选择当前命令块 |
| `mvi` | 选择当前命令输入 |
| `mvo` | 选择当前命令输出 |

## Workspace picker 基础

用以下命令打开 picker：

```bash
sivtr copy --pick
sivtr copy claude --pick
sivtr copy codex --pick
sivtr copy hermes --pick
```

Picker 包含 Source、Sessions、Dialogues 和 Content 面板。

| 按键 | 动作 |
| --- | --- |
| `0` / `1` / `2` / `3` | 聚焦 Source、Sessions、Dialogues 或 Content |
| `Space` | 切换当前 source、session 或 dialogue |
| `i` / `o` / `y` / `c` | 复制输入、输出、块或命令 |
| `/` | 搜索所有 session |
| `:` | 设置下一次复制用的一次性行过滤 |
| `t` | 打开 Vim 风格 full view |
| `z` | 当前面板全屏切换 |
| `?` | 帮助 |

完整按键见[快捷键](/zh-cn/reference/keybindings/)。

## 交给编辑器

在 browser 中按 `e`，会把当前选择内容（如果没有选择则是整个缓冲区）交给配置的编辑器。

配置编辑器：

```toml
[editor]
command = "nvim"
```
