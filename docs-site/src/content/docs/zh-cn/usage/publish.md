---
title: 发布浏览器只读对话链接
description: 用 Sivtr 把本地 Agent 对话安全地发布成浏览器可打开的只读链接。
---

`sivtr publish` 可以把本地的一段 Agent 对话变成一个浏览器链接。查看者不需要安装 Sivtr，也不需要登录；你的电脑关机后，链接仍然可以打开。

它发布的是一次性的“快照”，不是实时共享。对话之后再发生变化，旧链接不会自动更新，需要重新生成一个新链接。

## 先记住三件事

1. PowerShell 里的 `@share_ready` 要加引号：`'@share_ready'`。
2. `publish` 有两种选择模式：旧版全 record WorkSet 生成 schema v1，只发布连续整轮的 User/Assistant；`--pick` 进入原子选择器，生成 schema v2，可在同一个本地 Agent session 内选择任意原子和不连续片段。
3. 链接本身就是查看凭据。拿到完整链接的人都可以查看，不要把链接放到公开 issue 或不可信群聊中。

## 先配置 endpoint

`[publish].endpoint` 默认是空的。创建链接前必须写成你实际部署的 publication 服务地址，例如自建的 `https://share.hnnulwh.cn`，或能访问 Cloudflare 时的 Worker 域名。CLI 不会在多个后端之间自动切换。

```toml
[publish]
endpoint = "https://share.hnnulwh.cn"
```

## 第一步：确认 CLI 版本

先检查当前终端实际使用的是不是包含 `publish` 的版本：

```powershell
sivtr --version
sivtr publish --help
```

如果看到：

```text
error: unrecognized subcommand 'publish'
```

说明当前 `sivtr.exe` 太旧。升级后，`publish --help` 应该能看到 `preview`、`create`、`list`、`link` 和 `revoke`。

## 第二步：准备一段要分享的对话

`publish` 的输入是 WorkSet。最常见的流程是从一个 Codex session 取出一段连续轮次并保存：

```powershell
sivtr search codex/<session-id> --sort oldest --latest 50 --save share_ready --refs
```

把 `<session-id>` 换成实际的 session ID。例如：

```powershell
sivtr search codex/abc123 --sort oldest --latest 50 --save share_ready --refs
```

`--latest 50` 先取该 session 最近 50 轮（搜索在未指定 `--latest`/`--limit` 时默认只留 5 条）。`--sort oldest` 让保存下来的 WorkSet 按时间正序，方便预览；`publish` 自己也会再按 record index 排序。

保存成功后，WorkSet 名字就是 `share_ready`。在 PowerShell 中引用它时必须写成：

```powershell
'@share_ready'
```

如果已经有合适的 WorkSet，也可以使用它的名字；例如 `@review` 要写成 `'@review'`。

### 选择范围时的建议

尽量只选择真正需要分享的连续对话轮次。不要直接使用混合了多个 session 的 `@last`，否则发布时会被拒绝。也不要把终端日志、远程 workspace 或多个 Agent session 混在一起。带关键词的 BM25 搜索可能会跳过中间轮次，那种 WorkSet 不能直接 publish。

## 原子选择：预览时精确到 part

当一整轮里只有部分内容适合公开，或需要把工具输出、Skill、Thinking 一起纳入快照时，使用 `--pick`。它复用 workspace 浏览器的选择界面，不增加另一套选择语义：

```powershell
# 只预览，不保存选择
sivtr publish preview codex/<session-id> --pick --format human

# 选择后保存为可复用的 WorkSet
sivtr publish preview codex/<session-id> --pick --save share_ready

# 后续预览和创建都使用同一组原子 anchors
sivtr publish preview '@share_ready' --format human
sivtr publish create '@share_ready' --expires 7d --yes
```

`--pick` 只接受**单个本地 Agent session**。`codex/<session-id>` 是最直接的输入；不能选择 terminal、remote、group，也不能把不同 provider 或不同 session 拼在一起。`--save` 必须和 `--pick` 同时出现，保存的是精确的 part anchors，而不是一次新的全文搜索。

### 选择器里的操作

选择器从 Dialogues 视图打开。常用操作如下；按 `?` 可在当前版本查看完整帮助：

| 位置 | 操作 | 发布含义 |
| --- | --- | --- |
| Dialogues | `Space` 标记当前轮；`a` 切换全部轮次 | 选择整轮，展开为该轮的全部原子 |
| Content | `Space` 标记当前块；点击块左侧标记点 | 只选择标记的原子 |
| Content | `v` 选择块区间；`J` / `K` 翻到下一个 / 上一个已选轮次 | 支持轮内部分选择和跨页选择 |
| Content | `Tab` 切换 Input / Output；`Enter` 折叠或展开光标块 | 选择仍以完整原子为边界 |
| Dialogues | `Enter` 提交整轮选择；Content 用 `y` 提交当前或已标记块 | 返回预览并生成选择结果 |
| 任意视图 | `q` / `Esc` 退出选择器 | 取消本次预览，不保存选择 |

不要拖拽只覆盖半个原子的字符范围来“凑”选择。发布选择需要完整 block；只产生文本范围而没有完整 block anchor 时，会明确报 `publication selection is empty`，不会擅自扩大成整轮。

### 原子边界和排序

- User、Assistant、Skill、Thinking 各自是一个原子。
- 同一工具调用的 ToolCall 和 ToolResult 是一个不可拆分的 Tool 原子；选中其中一端会展开为完整的调用与结果。
- 父级 run 被选中时，会展开为其中全部子原子；不会只发布折叠标签。
- 可以跨轮次、跨页、非连续选择。Sivtr 会按原始 record index 和 part 顺序去重、排序；未选中的区间不会自动补入。
- 如果非连续选择前方存在未分享内容，schema v2 会设置 `gap_before: true`，查看器显示“部分内容未分享”，但不会暴露缺口数量或本地位置。

保存成功后，终端会显示 `saved @share_ready`。保存的 WorkSet 只包含选中的本地 anchors 以及这些 anchors 所属的 record，不会保留未选中轮次的正文；公开快照不会包含 WorkRef、session ID、record/part 序号、路径或 cwd。

### 什么时候会生成 v1 或 v2

| 输入 | 快照 schema | 内容 | 选择限制 |
| --- | --- | --- | --- |
| 全 record WorkSet（例如 `search --save`） | v1 | 连续整轮中的 User / Assistant | 同 provider、同 session、record index 连续 |
| part-anchor WorkSet（`--pick`） | v2 | User、Assistant、Tool、Skill、Thinking 原子 | 同 provider、同一个本地 session；允许非连续 |
| whole record 与 part anchor 混用 | 拒绝 | — | 明确报错，避免范围含义不确定 |

如果原子选择为空、工具调用缺少配对结果，或者保存的 WorkSet 后来被手工混入 whole/part 两种 anchor，预览和创建都会拒绝，不会生成“看起来完整但语义不确定”的链接。

## 第三步：本地预览

`preview` 完全在本地运行，不会上传内容：

```powershell
sivtr publish preview '@share_ready' --format human
```

预览会告诉你：

- 标题和来源 provider；
- 将发布多少轮对话和多少个公开项（v2 中公开项是原子，不等同于消息条数）；
- 将使用 schema v1 还是 v2；
- 快照大小和有效期；
- 自动脱敏了多少项；
- 是否发现路径、邮箱、内网地址等风险提示。

预览内容时重点检查：

- 是否包含不想公开的 User 消息或 Assistant 回复；
- 是否包含文件路径、邮箱、内网地址；
- 是否有不应该出现的密钥或账号信息；
- 对话起止范围是否正确。

识别出的 token、私钥、Bearer 和 secret assignment 会自动替换成 `[REDACTED]`。路径、邮箱和内网地址默认只警告，不会擅自改写正常对话。

### 查看器对 v1/v2 的显示

v1 的 User / Assistant 会按原顺序直接显示。v2 仍按原始时间顺序显示，但 Tool、Skill、Thinking 默认折叠，展开后可查看其脱敏后的 parts；Tool 原子会同时显示 call 和 result。非连续原子之间显示“部分内容未分享”。Markdown、链接和工具输出仍经过查看器的安全清理。

同一个已保存的原子 WorkSet，`preview` 和随后 `create` 的内容 SHA-256 会保持一致；发布时间和过期时间属于 envelope 元数据，不会改变内容哈希。

## 第四步：创建链接

确认预览没有问题后，创建一个 7 天有效的链接：

```powershell
sivtr publish create '@share_ready' --expires 7d --yes
```

有效期可以选择：

```text
1d    1 天
7d    7 天，默认值
30d   30 天
90d   90 天
```

没有永久链接选项。

如果风险报告里还有未自动处理的路径、邮箱或内网地址警告，**无论是否在交互终端**，都必须加上 `--allow-warnings`：

```powershell
sivtr publish create '@share_ready' --expires 7d --yes --allow-warnings
```

`create` 成功后，完整链接会输出到 stdout，方便复制。链接 host 来自 `[publish].endpoint`，通常类似：

```text
https://share.hnnulwh.cn/s/7d_xxxxxxxxxxxxxxxxxxxxxx#k=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

其中 `#k=...` 是解密密钥。它位于 URL fragment，不会发送给服务器；但是浏览器地址栏中的完整链接持有者可以查看内容。

### 在 PowerShell 中保存链接

如果想把链接保存到变量而不立即打印：

```powershell
$link = sivtr publish create '@share_ready' --expires 7d --yes --allow-warnings
$link
```

不要把 `$link` 写入公开日志、issue 或聊天记录。

## 查看、查找和撤销链接

查看本机发布记录，但不显示完整解密链接：

```powershell
sivtr publish list
```

使用 JSON 输出：

```powershell
sivtr publish list --json
```

需要重新打印某个完整链接时，使用 publication ID：

```powershell
sivtr publish link 7d_xxxxxxxxxxxxxxxxxxxxxx
```

提前撤销：

```powershell
sivtr publish revoke 7d_xxxxxxxxxxxxxxxxxxxxxx --yes
```

撤销会立即让链接不可访问。重复撤销同一条本机已撤销记录可以安全执行。管理凭据只保存在本机的 `publication-state.db` 中；如果这个数据库丢失，v1 没有账号恢复或远程找回管理权的功能。

## `publish` 和 `share` 的区别

| 功能 | `publish` | `share` |
| --- | --- | --- |
| 结果 | 不可变的浏览器只读快照 | 实时 workspace mount |
| 查看者 | 不需要 Sivtr，不需要登录 | 通常需要 Sivtr/daemon 和授权 |
| 分享者是否需要在线 | 不需要 | 通常需要 |
| 内容变化 | 不会自动更新，需创建新链接 | 读取共享 workspace 的当前内容 |
| 服务端看到的内容 | 只有加密密文 | 按远程共享协议提供数据 |

## 数据会不会经过你的服务器？

不会把完整的本地 WorkSet 原样上传，但 `publish create` 也不是“完全离线”。准确的过程是：本地生成和加密，服务器保存密文，查看者浏览器解密。

```text
本地 WorkSet
  ↓ 本地筛选、脱敏、生成公开快照
  ↓ 本地 AES-256-GCM 加密
你的服务器
  ↓ 返回加密 envelope
查看者浏览器
  ↓ 使用链接 fragment 中的密钥解密
显示 User / Assistant 对话
```

### 哪些步骤只在本地发生

- `sivtr publish preview` 完全在本地运行，不联网；
- WorkSet materialize、连续轮次校验和敏感信息扫描在本地完成；
- 原始对话、WorkSet、WorkRef、`cwd`、session path 不会上传；
- 明文快照先在本地压缩，再用独立的 AES-256-GCM 密钥加密。

### 服务器会保存什么

你的 `share.hnnulwh.cn` 服务器会收到并保存加密后的 envelope，另外保存发布所需的期限、创建时间、版本和管理 token 哈希。服务器不能从这些内容还原对话，也不会保存标题、provider 或来源 refs。

查看链接时，浏览器会从服务器取回密文。`#k=...` 位于 URL fragment，不会随 HTTP 请求发送给服务器；真正的解密发生在查看者浏览器中。因此分享者电脑可以关机，但查看者打开链接时服务器仍需在线。

服务器及 Nginx 仍可能知道：

- publication ID、请求时间、IP、状态码和响应大小；
- 某个链接被创建、查看或撤销；
- 到期时间和密文文件本身。

应用日志不会记录请求体、管理 token、fragment 密钥或解密内容；但服务器上的 Nginx 默认访问日志仍可能记录请求路径和访问 IP。

### 需要注意的安全边界

1. **完整链接就是查看凭据。** 拿到完整链接的人不需要登录即可查看。浏览器历史、剪贴板、聊天软件同步、截图和浏览器扩展都可能造成链接泄露。
2. **服务器被攻破时仍有 Viewer 完整性风险。** 正常服务器拿不到 fragment 密钥；但如果攻击者能替换服务器上的 Viewer JavaScript，就可能读取浏览器地址栏中的密钥。因此这是应用层加密，不代表能抵御已经被入侵的服务器。
3. **本机状态库保存密钥。** `publication-state.db` 保存查看密钥和撤销用的管理 token，不保存公开快照明文。能读取你 Sivtr 数据目录的人可能重新取得链接或执行撤销。
4. **脱敏不是绝对保证。** 已知 token、私钥、Bearer 和 secret assignment 会自动替换；路径、邮箱、内网地址只警告；未识别的敏感内容仍可能进入快照。

### 实用建议

- 高敏感对话优先使用 `--expires 1d`；阅读完成后立即撤销；
- 不要把完整链接写入公开 issue、公共群聊或公开日志；
- 保护 Windows 用户账户和 `publication-state.db`，不要将其同步到公开云盘或代码仓库；
- 保持 HTTPS 证书、Nginx 和 Node 服务更新，并定期轮换、清理访问日志；
- 第一次使用时先发布不含敏感信息的测试对话。

## 内容边界：v1 与 v2

全 record 输入仍按 v1 处理：只支持同一 provider、同一 session 中连续的本地 Agent 对话轮次，并只保留 User 和 Assistant 文本。使用 `--pick` 保存 part anchors 后，v2 可以公开更多原子类型，但以下边界对两个版本都有效：

- Terminal 记录；
- remote/group 内容；
- 跨 session 或跨 provider 内容；
- WorkSet、WorkRef、`cwd`、session path；
- provider 原始事件、附件和图片。

在 v1 中，ToolCall、ToolResult、Thinking、Skill 不进入公开快照；在 v2 中它们分别投影为 Tool、Thinking、Skill 原子（ToolCall 与 ToolResult 必须成对）。所有原子内容经过同一套凭据脱敏、路径/邮箱/内网地址风险检查。

## 常见错误

### `unrecognized subcommand 'publish'`

当前终端使用的是旧版 `sivtr.exe`。执行：

```powershell
sivtr --version
Get-Command sivtr -All
```

确认 PATH 中实际使用的二进制已经升级，并重新打开一个 PowerShell 窗口。

### `[publish].endpoint is not set`

还没有配置 publication 服务地址。在 `config.toml` 里设置 `[publish].endpoint` 后再创建链接。

### `failed to resolve publication source '@share_ready'`

本机没有名为 `share_ready` 的 WorkSet。先执行搜索并保存：

```powershell
sivtr search codex/<session-id> --sort oldest --latest 50 --save share_ready --refs
```

PowerShell 里不要省略 `'@share_ready'` 的引号。

### `publication cannot mix agent sessions`

你选择的 WorkSet 包含多个 Agent session。缩小搜索范围，只选择一个 session 中连续的对话轮次，再重新保存 WorkSet。

### `publication record indices must be strictly continuous`

WorkSet 里的轮次排序后仍有缺口（例如关键词搜索跳过了中间几轮）。改成按 session 取一段连续窗口：`--sort oldest --latest N`，不要混入不相关命中。

### `publish preview --pick requires an interactive terminal`

`--pick` 需要真实交互式终端。脚本、管道、CI 或重定向环境不能打开选择器；在交互 PowerShell 中执行，再把保存的 WorkSet 交给后续自动化创建。

### `publish preview --save requires --pick`

`--save` 只用于保存原子选择，不能单独使用。要保存整轮搜索结果，请使用：

```powershell
sivtr search codex/<session-id> --sort oldest --latest 50 --save share_ready --refs
```

### `publication selection is empty`

选择器没有收到完整 dialogue 或 block anchor。回到 Dialogues 用 `Space` 选择整轮，或在 Content 用 `Space` / `v` 标记完整块；不要只拖选半个原子的字符。

### `publication selection must include complete tool atoms`

工具调用和结果不可拆分。重新选择该工具块的完整 call/result；选择器通常会在选中一端时自动展开另一端，手工修改 WorkSet 时则必须保留两端。

### `publication cannot mix whole-record and part anchors`

同一个 WorkSet 不能同时表达“整轮”和“原子片段”。重新保存一个纯整轮 WorkSet（走 v1）或纯 part-anchor WorkSet（走 v2）。

### `publication picker requires exactly one local agent session`

原子选择器一次只处理一个本地 Agent session。不要把 terminal、remote/group、不同 provider 或多个 session 的记录放进 picker 输入。

### `non-interactive publish requires --yes`

脚本或重定向环境不是交互终端，创建时加上：

```powershell
--yes
```

### `publish with privacy warnings requires --allow-warnings`

预览仍有未自动处理的路径、邮箱或内网地址。看完预览并确认可以公开后，显式加上：

```powershell
--allow-warnings
```

交互终端里只回答确认提示是不够的；有警告时必须带这个 flag。

## 隐私和数据位置

本机的 `publication-state.db` 保存标题、来源摘要、期限、查看密钥和撤销凭据，但不保存公开快照明文。

服务器只保存加密后的 envelope 和撤销/过期所需的元数据。服务器不能从 URL 请求中得到 `#k=...` fragment，也不会生成标题预览或搜索引擎内容。

如果对话包含高敏感内容，建议使用更短的 `1d` 有效期，并在确认查看者完成阅读后主动撤销。

更多精确参数见 [CLI 参考](/zh-cn/reference/cli/)，配置说明见 [配置](/zh-cn/usage/configuration/)。
