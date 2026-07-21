---
title: 浏览和选择
description: 导航 workspace 浏览器与单缓冲浏览器，选择文本并复制。
---

`sivtr` 有两个交互界面：

- **workspace 浏览器**（TTY 下裸 `sivtr`，或 `sivtr copy --pick` / 热键）：多源 Source → Sessions → Dialogues → Content；
- **单缓冲浏览器**（管道 stdin 或 `sivtr run` / `sivtr pipe`）：单个捕获输出缓冲区。

## 打开 workspace 浏览器

```bash
sivtr                     # TTY：多源 workspace 浏览器
sivtr --all               # 打开时也选中 remote mount
sivtr copy --pick         # 同一浏览器，面向复制
sivtr copy claude --pick
```

布局：Source · Sessions · Dialogues · Content。Content 拆成独立滚动的 **Input** / **Output** 半窗。

### Workspace 导航

| 按键 | 动作 |
| --- | --- |
| `0` / `1` / `2` / `3` | 聚焦 Source、Sessions、Dialogues 或 Content |
| `h` / `l` | 上一 / 下一面板 |
| `j` / `k` | 下移 / 上移 |
| `Space` | 切换选择（source / session / dialogue） |
| `a` | 全选 source（Source）· 切换全部 dialogue（Dialogues） |
| `g` / `t` | 选 agent 源 / terminal 源（Source） |
| `R` | 刷新活动行下一级 |
| `v` | Dialogue 范围选择 · Content 视觉选字 |
| `Tab` | Content 半窗 Input ↔ Output |
| `r` | 折叠 / 展开结构标记与完整载荷 |
| `Ctrl-d` / `Ctrl-u` · `PgDn` / `PgUp` | 滚动 Content |
| `g` / `G` | Content 顶 / 底 |
| `i` / `o` / `y` / `c` | 复制输入 / 输出 / 块 / 命令 |
| `Enter` | 确认 / 打开下一级 / 复制 |
| `/` | 搜索 |
| `z` | 当前面板全屏 |
| `t` | Vim 风格 full view（Sessions/Dialogues） |
| `?` | 帮助 |
| `q` / `Esc` | 退出 / 返回 |

鼠标：Content 上拖选文本；`Ctrl`-拖为块选。Source 聚焦时纵向展开，失焦为紧凑条。Content 半窗高度偏向当前焦点半窗。

结构 part（tool / skill / thinking）在折叠模式显示为 `<:channel:…:>` 标记；`r` 展开完整载荷。

完整按键见[快捷键](/zh-cn/reference/keybindings/)。

## 单缓冲浏览器

管道捕获或 `sivtr run` 打开 Vim 风格只读浏览器，浏览单个缓冲区。

| 按键 | 动作 |
| --- | --- |
| `j` / `k` · 方向键 | 移动 |
| `Ctrl-D` / `Ctrl-U` | 半页 |
| `Ctrl-F` / `Ctrl-B` · Page 键 | 整页 |
| `gg` / `G` | 顶 / 底 |
| `/` · `n` / `N` | 搜索 · 下一 / 上一匹配 |
| `v` / `V` / `Ctrl-V` | 字符 / 行 / 块选择 |
| `y` | 复制选择 |
| 鼠标拖 · `Ctrl`-拖 | 选字 · 块选 |
| `e` | 把选择（或整缓冲）交给配置的编辑器 |
| `[[` / `]]` | 上一 / 下一命令块（session log） |
| `myy` / `myi` / `myo` / `myc` | 复制块 / 输入 / 输出 / 裸命令 |

配置编辑器：

```toml
[editor]
command = "nvim"
```
