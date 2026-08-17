# SIVTR 资源引用（WorkRef / URI）

## 一、设计目标

SIVTR 用一套引用字符串定位「某次工作里的一条记录 / 一行 / 一块内容」，并在本地、其它 workspace、远程 mount 之间保持同一语法。

要求：

1. **简洁**：日常命令默认省略当前 workspace。
2. **可定位**：能指到 session 内的一条工作、一行、一块 part。
3. **可路由**：左侧说明资源在哪（scope），右侧说明 scope 内路径与落点（path + at）。
4. **可扩展**：scope 可接 mount 别名、本机 workspace、短别名与上下文默认值；新 source / 寻址方式扩在 path 上。
5. **职责分离**：精确 ref、搜索 selector、`@WorkSet` 各司其职；实体元数据不进入 URI。

---

## 二、总形态

```text
[scope:]path[/at]
```

| 段 | 含义 | 例子 |
| --- | --- | --- |
| `scope` | 资源在哪 | `desk`、`docs`、`alice/sivtr`、`&ahs` |
| `path` | scope 内哪一条工作（加载后对应一个 `WorkRecord`） | `terminal/session_42/3`、`codex/abc123/5` |
| `at` | 落在该条上的哪一截 | （省略=整条）、`/p1`、`/p2` |

冒号只切一次。左侧是 scope，右侧是 `path` 与可选的 `at` 后缀。

> **`path` 不是文件系统路径。** 它是 scope 内的逻辑路径：`source/session/index`。会话文件在磁盘上的位置属于实体元数据，不进 URI。

```text
codex/4                        # 当前 workspace，整条
docs:codex/4                   # 本机另一 workspace
desk:terminal/session_42/3/p1 # mount + part
team/alice:terminal/session_42/3/p1 # group/member + part
&ahs:codex/4                    # 用户短别名 → 完整 scope
```

### 分隔符

- **只用 `:`**。
- **禁止 `://`**（不是 URL，也不要伪装成 scheme）。
  - `desk://terminal/...` → 非法，应提示使用 `desk:terminal/...`。

### 解析与显示

- 无 `:` → 当前 workspace（local）。
- `local:…` → 与 bare 等价；**显示时写回 bare**（不保留 `local:` 前缀）。
- 其它 `scope:…` → 非当前 scope；显示保留 `scope:`。
- `at = Whole` 时不写后缀；`Line` / `Part` 写在 path 后面。

---

## 三、三维模型

`WorkRef` 固定三层。类型名统一 `Work*`；字段名用短词。

```text
WorkRef {
  scope: WorkScope,   // 在哪套索引
  path:  WorkPath,    // 哪一条 = source/session/index
  at:    WorkAt,      // 落在哪一截
}
```

同域类型（地址 vs 实体分清）：

```text
地址族:  WorkRef / WorkScope / WorkPath / WorkAt / WorkRefSelector
实体族:  WorkRecord / WorkPart / WorkPartKind / WorkSet
```

```text
WorkScope（在哪）
  Local | Named("desk" | "docs" | "alice/sivtr" | …)
        │
        │  scope:path[/at]
        ▼
WorkRef
  scope + path + at
        │
        ├─ path ───────────────────────────────┐
        │                                      ▼
        │                         WorkPath
        │                           Terminal { session, index }
        │                           Agent    { provider, session, index }
        │                           （扩展时只加变体，见下）
        │
        └─ at ────────────────────────────────┐
                                              ▼
                                    WorkAt
                                      Whole
                                      Part(seq)
```

加载之后（**不是 URI 的第四维**）：

```text
WorkRecord                    # 实体：title、parts、time、status…
  work_ref: WorkRef           # 回指自己的地址
  session:  { id, path, … }   # 会话元数据（文件路径等）；不进 URI
  parts: WorkPart[]
```

| 概念 | 类型 | 字段名 | 说明 |
| --- | --- | --- | --- |
| 精确地址 | `WorkRef` | — | `scope` + `path` + `at` |
| 资源空间 | `WorkScope` | `scope` | `Local` 或 `Named(String)` |
| 逻辑路径 | `WorkPath` | `path` | `source/session/index`；打开一条 `WorkRecord` |
| 落点 | `WorkAt` | `at` | `Whole` / `Part(seq)` |
| 工作实体 | `WorkRecord` | — | 加载结果，不是地址 |
| 内容块 | `WorkPart` | — | 实体内的一块文本 |
| 搜索范围 | `WorkRefSelector` | — | 可缺段 / `*`（不是精确 ref） |
| 结果面 | `WorkSet` | — | records + anchors（anchor 是 `WorkRef`） |

**`WorkPath` ≠ `WorkRecord`：**

- `WorkPath`：scope 内的逻辑路径 / 身份，可放进地址、可比较「是不是同一条」。
- `WorkRecord`：加载后的完整数据。

会话文件路径、canonical id 等只挂在实体上，**不进入** `WorkRef` / `WorkPath` 字符串。

### 操作与三层

| 层 | 类型 | 关心谁 | 典型操作 |
| --- | --- | --- | --- |
| `scope` | `WorkScope` | 路由 | `load_source`、remote fetch |
| `path` | `WorkPath` | 索引里哪一条 | resolve → `WorkRecord` |
| `at` | `WorkAt` | 实体内切片 | show part、copy 子块 |

```text
ref.with_part(2)  → at = Part(2)，scope/path 不变
ref.whole()       → at = Whole
```

### 为什么 path 要包装成类型

`source/session/index` 必须有类型名，否则「三段」在类型上会散成一堆字段，扩展也没落点：

| 扩展 | 落在哪 |
| --- | --- |
| 新 source（web chat、CI…） | **`WorkPath` 新变体** |
| session 写法、`current`、短 id | **`WorkPath` 解析** |
| 非序号寻址（按 id） | **`WorkPath` 新形态** |
| 新 mount / 别名 | `WorkScope` |
| 新切片方式 | `WorkAt` |

对外仍尽量传完整 `WorkRef`；`WorkPath` 是 `WorkRef` 的字段类型，不是第二套平行地址。

---

## 四、WorkScope

Scope 是逻辑资源空间，不是文件系统路径。它回答「从哪套索引读」。

### 1. 形态

```text
name
group/member
group/member/share
```

规则：

- 一至三段，用 `/` 连接；**最多三段**。
- 每段 `[A-Za-z0-9_-]+`，大小写不敏感，**规范化为小写**。
- **`local` 保留**：表示当前 workspace；写成 `local:…` 与 bare 相同。
- 未注册 / 无法解析的 scope → **报错**，不探测网络、不猜 host。

| 形态 | `WorkScope` | 例子 |
| --- | --- | --- |
| 省略（bare） | `Local` | `terminal/session_42/3` |
| `local` | `Local` | `local:codex/4` → 显示为 `codex/4` |
| 单段名 | `Named("docs")` / `Named("desk")` | `docs:codex/4`、`desk:terminal/...` |
| `group/member` | `Named("team/alice")` | `team/alice:terminal/...` |
| `group/member/share` | `Named("team/alice/proj-b")` | `team/alice/proj-b:codex/4` |

解析顺序：mount 别名（workspace 内，单段）→ group（device 全局，1-3 段）→ 本地 workspace origin（单段，目录 basename 小写）。两段以上不会被当作本地 workspace；`device/workspace` 形态已由 `group/member` 取代（远程 mount 只用单段别名）。

本机可用名见 `sivtr ws list`。远程 mount 由 `sivtr remote add <alias> <invite>` 注册；alias 出现在 scope 槽。

### 2. 短别名 `&alias`

```text
desktop/ai-help-study  →  &ahs
web/openai-personal    →  &gpt
server/production      →  &prod
```

```text
&ahs:claude/s123/3
&gpt:chatgpt/c456/8
&prod:terminal/s789/4
```

- `&` 仅在 scope 侧；展开后与普通 scope 同路径。
- 未定义 alias → 明确错误。
- 稳定对外形式写完整 scope；`&alias` 是输入糖。
- 配置：`config.toml` → `[scope].aliases`；CLI：`sivtr alias set|list|remove`。
- `load_source` / `copy ref` 解析前调用 `expand_source`（只展开 `&alias` / `local:`）。

---

## 五、WorkPath

`path` 是 scope 内的逻辑路径，指名「哪一条工作」，加载后对应一个 `WorkRecord`。**不含**行号或 part。

### 1. 总形

```text
<source>/<session>/<index>
```

| 段 | 含义 |
| --- | --- |
| `source` | `terminal`，或 agent provider 命令名 |
| `session` | session id 或稳定缩写（如 `current`） |
| `index` | **1-based** 序号（terminal record / agent turn） |

### 2. Terminal

```text
terminal/<session>/<index>
```

```text
terminal/session_42/3
terminal/current/1
```

```text
WorkPath::Terminal { session, index }
```

### 3. Agent

```text
<provider>/<session>/<index>
```

`provider` 必须是已注册的 `AgentProvider` 命令名。

```text
codex/abc123/5
claude/s9f2/2
pi/abcdef12/2
```

```text
WorkPath::Agent { provider, session, index }
```

### 4. 不进入 path 的东西

- part → **`WorkAt`**
- `WorkPart.kind` → 命令 flag（如 `--kind`），不写进 URI
- 范围、通配、标签、`latest` → selector / filter
- 会话文件 path、canonical id → 仅实体侧元数据

---

## 六、WorkAt

`at` 表示落在已解析 `WorkRecord` 上的哪一截。

| 写法 | `WorkAt` | 含义 |
| --- | --- | --- |
| （省略） | `Whole` | 整条 |
| `/p<n>` | `Part(n)` | 第 n 个 `WorkPart`（1-based，跨输入/输出统一编号） |

- part 序号是全记录 1-based index（`p1` 是第一个 part，不区分输入/输出侧）
- kind 不进入 path 字符串

```text
terminal/session_42/3           # Whole
terminal/session_42/3/p1        # Part(1)
codex/abc123/5/p2               # Part(2)
desk:codex/abc123/5/p2          # scope + path + at
```

精确 ref **不包含**：未定义 branch 字母、范围、`*`、`@tag`、`latest`。

---

## 七、段解剖

```text
desk : codex / abc123 / 5 / p2
────   ─────   ──────   ─   ──
scope  source  session index part
       └────── WorkPath ─────┘ └ WorkAt ┘
```

| 字面 | 层 |
| --- | --- |
| `desk` | `WorkScope::Named("desk")` |
| `codex/abc123/5` | `WorkPath::Agent { Codex, "abc123", 5 }` |
| `p2` | `WorkAt::Part(2)` |

---

## 八、精确 Ref vs Selector vs WorkSet

| 形式 | 用途 | 例子 |
| --- | --- | --- |
| **WorkRef** | 唯一地址 | `desk:codex/abc/5/p2` |
| **Selector** | 搜索范围，可缺段 | `terminal`、`desk:agent`、`codex/*/3` |
| **WorkSet 变量** | 物化结果集 | `@last`、`@hits[1,3..5]` |

```bash
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr s desk:agent -m "panic|failed" --save remote_hits
sivtr show desk:terminal/session_42/3/p1 --full
```

`@name` 不是 scope；不要写成 `scope:@name`。

---

## 九、命令面（与引用相关）

```bash
sivtr show claude/s123/3
sivtr show desk:terminal/session_42/3/p1
sivtr show &ahs:codex/4

sivtr s terminal --status failure
sivtr s desk:agent -m "decision" --latest 20 --refs
sivtr s &prod:terminal

sivtr alias set ahs desktop/ai-help-study
sivtr alias list
sivtr alias remove ahs

sivtr ws list
sivtr remote add desk <key>
sivtr remote list
sivtr share
```

---

## 十、端到端例子

```text
输入:  desk:codex/abc123/5/p2

WorkRef {
  scope: Named("desk"),
  path:  Agent { provider: Codex, session: "abc123", index: 5 },
  at:    Part(2),
}

加载: desk 索引 → WorkRecord → 第 2 个 WorkPart
显示: desk:codex/abc123/5/p2
```

```text
输入:  terminal/session_42/3

WorkRef {
  scope: Local,
  path:  Terminal { session: "session_42", index: 3 },
  at:    Whole,
}
显示:  terminal/session_42/3
```

```text
输入:  local:terminal/session_42/3
语义:  同上（WorkScope::Local）
显示:  terminal/session_42/3
```

```text
输入:  &ahs:claude/s123/3/p1

scope = Named(展开 &ahs)
path  = Agent { Claude, "s123", 3 }
at    = Part(1)
```

---

## 十一、禁止

| 写法 | 状态 |
| --- | --- |
| `desk:terminal/...` | 现行 |
| bare `terminal/...` | 现行（当前 workspace） |
| `local:terminal/...` | 现行（等价 bare，显示 bare） |
| `desk://…`、`local://…` | **禁止** |
| host:port 写入 scope | **禁止** |
| 精确 ref 写 kind / 未定义 branch | **禁止** |
| `/i/<n>`、`/o/<n>`、`/<n>`（行）part/line 后缀 | **已移除**（旧版 i/o 与 line 形式；现行用 `/p<n>`） |

---

## 十二、设计原则

1. **`[scope:]path[/at]`**，冒号切一次；不用 `://`。
2. **`WorkRef = WorkScope + WorkPath + WorkAt`**，三层正交；字段名 `scope` / `path` / `at`。
3. **WorkScope** = `Local` \| `Named`（`name`、`group/member` 或 `group/member/share`，≤3 段）。
4. **WorkPath** = `source/session/index` 的包装类型；是 scope 内逻辑路径，**不是**文件路径，**不是**实体 `WorkRecord`。扩展 source / 寻址只动这一层。
5. **WorkAt** = `Whole` \| `Part(seq)`（`/p<n>`）。
6. **完整 scope 保唯一，`&alias` 保输入效率**。
7. **精确 ref / selector / `@WorkSet` 三分**。
8. **地址族与实体族分离**；会话文件 path 等元数据不进 URI。

最终形态：

```text
scope:source/session/index[/at]
```

日常最短：

```text
codex/4
&ahs:codex/4
desk:terminal/...
```
