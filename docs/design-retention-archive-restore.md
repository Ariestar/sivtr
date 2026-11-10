# 保留、归档与恢复：三层存储

| 字段 | 值 |
| --- | --- |
| 标题 | 保留、归档与恢复：三层存储 |
| 作者 | TBD |
| 日期 | 2026-11-04 |
| 状态 | 草稿 |
| 产品 | sivtr ([github.com/Ariestar/sivtr](https://github.com/Ariestar/sivtr)) |
| 北极星 | 会话正文在本地只存一份、可校验、可重建；删除可逆，空间回收可审计 |

---

## 概述

`sivtr` 现在把 `cache/archive.db` 同时当作索引、正文仓库和热缓存。三种职责压在一张派生表上，带来三个后果：原生日志消失后 `remove_missing_sessions` 连同正文一起删行；`starred` 这类用户意图存在派生表里，schema 升级会 DROP 掉；正文在 `blob`、`blob_light` 和每个 `sets/*.json` 里各存一份，且未压缩。

本设计把存储拆成三层，每层只有一个职责：

| 层 | 位置 | 内容 | 可重建 |
| --- | --- | --- | --- |
| 冷层 | `<home>/store/objects/` | 内容寻址的 zstd 压缩正文，每段文本一份 | 否，是唯一副本 |
| 元数据 | `<home>/meta.db` | 引用 → 哈希、时间、来源、workspace、标记、墓碑、副本位置 | 否，体积以 MB 计 |
| 热缓存 | `<home>/cache/` | 最近会话正文、BM25、listing | 是，随时可删 |

删除拆成两步，和 restic / borg 一致：`forget` 只改元数据，可逆；`prune` 才回收空间，不可逆。两步都先打印计划，`--apply` 才执行。恢复 = 校验哈希 → 取对象 → 解压 → 租进热缓存。

---

## 背景

### 当前形态（2026-11-04，本机实测）

| 路径 | 内容 | 大小 |
| --- | --- | --- |
| `cache/archive.db` | 767 个会话 / 5971 条记录的正文与索引 | 364.7 MiB |
| `sets/*.json` | 74 个 WorkSet，含全量记录正文 | 205.9 MiB |
| `cache/bm25-*.bin` | 8 个可重建的 BM25 索引 | 99.6 MiB |
| `cache/listing-*.bin` | 874 个会话列表缓存 | 0.7 MiB |
| `workspaces/` | 终端 capture 日志 | 5.8 MiB |
| 其他（config、state、daemon） | | 0.13 MiB |
| 合计 | | 676.9 MiB |

实测压缩率：全部 blob 用 gzip-6 可到 3.27×；最大的 WorkSet `rebase_context.json` 31.8 MiB 可到 4.82×。带 ANSI 的终端记录里，原文与 `*_ansi` 两份字符串让该段膨胀 1.84–1.88×。

### 五个缺口

1. **archive 是唯一副本。** `remove_missing_sessions`（`store.rs:563`）按 `source_path` 删除源文件已消失的行，唯一豁免是 `capture://` 前缀。Codex / Claude 清理自己的日志后，sivtr 的记忆随之消失。已确认过一次：根目录下的旧 archive 删除后，481 个源文件已不存在的会话永久丢失（Codex 455 个 / 338.0 MB）。
2. **用户意图在派生表上。** `starred` 是 `sessions` 的列，由 `ensure_column` 补齐；`SCHEMA_VERSION` 变化时整表 DROP 重建，星标归零。`sync.rs` 里还有一条迁移测试专门断言 Cursor 的星标能活下来，说明这个不变量已经被依赖。
3. **正文重复。** 同一段文本在 `records.blob`、`records.blob_light`（去掉 part 文本的元数据视图）、以及每个引用它的 WorkSet JSON 中各存一份。正文没有第二份独立用途。
4. **不压缩。** SQLite 按原样存 MessagePack，3.27× 的压缩空间没有使用。
5. **没有保留策略，也没有完整性检查。** 没有 `keep-*` 规则、没有副本计数、没有哈希校验；`doctor` 不检查 archive 是否完好。

---

## 成熟做法

这套问题在备份工具里已经收敛出稳定做法。我们采用机制，不引入它们的部署复杂度。

| 系统 | 机制 | 采用的部分 |
| --- | --- | --- |
| restic | `forget` 只从索引移除快照，`prune` 删除无引用数据；`--keep-last/-daily/-weekly/-monthly/-yearly/-within`；`check --read-data` 重算哈希 | 删除分两步；保留桶；抽样与全量校验 |
| borg | `prune` 按 keep 规则删归档，`compact` 单独回收段空间 | 回收空间是独立动作 |
| git | reflog 默认保留 90 天，不可达对象 30 天后过期，`gc.pruneExpire` 默认 2 周 | 墓碑宽限期；过期前可恢复 |
| git-annex | `numcopies` 是期望副本数，`mincopies` 是硬下限；`drop` 在低于下限时拒绝，`--force` 显式覆盖；`whereis` 列出位置 | 副本计数；不足时拒绝 `prune`；恢复前先报告在哪些位置找过 |
| S3 lifecycle | 热 → 温 → 冷分层；非当前版本过期；Glacier 恢复产出带期限的临时副本 | 有界热窗口；恢复是租约 |
| S3 Object Lock | retention 与 legal hold 阻止删除 | 星标与保存的 WorkSet 作为保留底线 |
| SQLite | `VACUUM` 逐页重写，需要独占；WAL 先 checkpoint | 回收后 compact，复用现有的 `storage vacuum` |

刻意不采用：内容定义分块去重、纠删码、WORM 合规锁、跨区域复制。它们解决的是 PB 级或多租户合规问题，本地工作记忆用不上。

---

## 目标形态

```text
写路径
  native log ──parse──> WorkRecord ──encode──> store/objects/ab/<sha256>.zst
                                            └─> meta.db 引用与元数据

读路径
  ref ──> meta.db ──> 热缓存命中？ ──是──> cache/archive.db ──> WorkRecord
                          │
                          否
                          └──> 冷层解压 ──> 租约写入热缓存 ──> WorkRecord
```

### 冷层

- 对象名是解压后规范字节的 SHA-256，按前两位分目录：`store/objects/ab/<hash>.zst`。
- 规范字节是现有的 MessagePack `WorkRecord` 编码，哈希覆盖解压后的内容；压缩格式可换而不影响寻址。
- zstd level 19。写入顺序：临时文件 → 写满 → 重新读取并校验哈希 → `rename`。同哈希已存在就跳过写入。
- 对象不可变。相同哈希意味着相同字节，重复写入是幂等操作。
- 对象数量增长到十万级后再引入 pack 文件（git / restic 的下一步）；在此之前散对象更容易校验和修复。
- `store/version` 记录存储格式版本。格式升级走显式迁移，不做双读。

### 元数据

`meta.db` 是普通 SQLite，体积目标是 home 的 1% 以内。核心表：

```sql
CREATE TABLE objects (
    hash        TEXT PRIMARY KEY,
    codec       TEXT NOT NULL,
    raw_size    INTEGER NOT NULL,
    stored_size INTEGER NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE records (
    provider    TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    idx         INTEGER NOT NULL,
    title       TEXT NOT NULL,
    started_at  TEXT,
    ended_at    TEXT,
    outcome     TEXT,
    exit_code   INTEGER,
    object_hash TEXT NOT NULL REFERENCES objects(hash),
    PRIMARY KEY (provider, session_id, idx)
);

CREATE TABLE sessions (
    provider     TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    source_path  TEXT,
    cwd          TEXT,
    workspace_key TEXT NOT NULL DEFAULT '',
    title        TEXT,
    started_at   TEXT,
    ended_at     TEXT,
    record_count INTEGER NOT NULL DEFAULT 0,
    project      TEXT NOT NULL DEFAULT '',
    mtime_secs   INTEGER NOT NULL DEFAULT 0,
    mtime_nanos  INTEGER NOT NULL DEFAULT 0,
    size         INTEGER NOT NULL DEFAULT 0,
    synced_at    TEXT NOT NULL,
    PRIMARY KEY (provider, session_id)
);

CREATE TABLE session_stars (
    provider   TEXT NOT NULL,
    session_id TEXT NOT NULL,
    starred_at TEXT NOT NULL,
    PRIMARY KEY (provider, session_id)
);

CREATE TABLE tombstones (
    provider     TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    forgotten_at TEXT NOT NULL,
    purge_after  TEXT NOT NULL,
    reason       TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (provider, session_id)
);

CREATE TABLE copies (
    object_hash TEXT NOT NULL REFERENCES objects(hash),
    location    TEXT NOT NULL,          -- local / desk / docs / export:<id>
    verified_at TEXT NOT NULL,
    PRIMARY KEY (object_hash, location)
);
```

`records` 的列已经覆盖 Light 模式需要的全部字段，`blob_light` 因此消失：列表与按时间过滤走纯 SQL，不再解码 MessagePack。

### 热缓存

- `cache/archive.db` 只保留热窗口内的正文，其余行保留元数据。窗口默认 30 天或 256 MiB，先到者生效，按 `last_accessed` 做 LRU。
- 热缓存是可重建状态：删除它只会让下一次读取变慢，不会丢内容。
- `listing-*.bin` 加上界与代数键（archive 每次重建换代，旧文件直接失效）；BM25 已有 8 个文件的上界，沿用。

---

## 生命周期

```text
        写入                冷层已有对象
  new ──────> hot ──────────────> cold
               │                   │
               │ forget            │ forget
               v                   v
           tombstoned <────────────┘
               │
               │ prune（宽限期已过 + 副本达标 + --apply）
               v
            deleted

  restore: tombstoned / cold ──> hot（租约）
```

| 状态 | 正文位置 | 列表可见 | 可读 | 可恢复 |
| --- | --- | --- | --- | --- |
| `hot` | 热缓存 + 冷层 | 是 | 是 | 不需要 |
| `cold` | 冷层 | 是 | 是（首次读取慢） | 不需要 |
| `tombstoned` | 冷层 | 是，带删除标记 | 是 | 是，清除墓碑 |
| `deleted` | 无 | 否 | 否 | 否 |

`forget` 写墓碑并把会话移出默认列表。超过宽限期后 `prune` 才删除对象。

---

## 删除

### forget（可逆）

```bash
sivtr storage forget codex/abc123           # 单个会话
sivtr storage forget terminal --before 2026-01-01
sivtr storage forget --policy default       # 按保留策略选出候选
```

先打印计划：会话数、记录数、对象数、可回收字节、被底线规则挡下的条目及原因。`--apply` 才写墓碑。宽限期默认 30 天，配置项 `[storage] forget_grace_days`。这对应 git 的不可达对象 30 天过期。

保留底线，优先级从高到低：

1. `session_stars` 中的会话。
2. 被任一已保存 WorkSet 引用的会话（`sets/*.json` 的 anchors 是显式意图）。
3. `--keep-within` 覆盖的时间窗。
4. 最新 N 个会话（默认 20）。

底线永远优先于 `--keep-*` 规则，也不被 `--force` 绕过；要删星标会话，先 `unstar`。

### prune（不可逆）

```bash
sivtr storage prune --dry-run    # 默认就是 dry-run
sivtr storage prune --apply
```

执行顺序：

1. 选出 `purge_after < now` 的墓碑。
2. 对每个待删对象统计 `copies`。`mincopies` 默认 2；不足时跳过并报告缺口，`--force` 可以显式覆盖（git-annex 的 `drop --force` 语义）。
3. 删除对象文件，再删元数据行。
4. `PRAGMA wal_checkpoint(TRUNCATE)`，然后 `VACUUM`；沿用 `storage.rs` 里已有的独占检查与忙碌错误。

`mincopies` 统计的是独立存储位置：本地冷层算一个，每个挂载的 peer 算一个，导出的 publication 算一个。同一磁盘上的热缓存不算副本。只有一个位置时，永久删除需要显式 `--force`，这与“唯一副本不删”的直觉一致，同时给单机用户留出出口。

### 保留策略

规则语法沿用 restic / borg：

```bash
sivtr storage forget --keep-within 14d \
                     --keep-last 20 \
                     --keep-daily 14 \
                     --keep-weekly 8 \
                     --keep-monthly 12 \
                     --keep-yearly 3
```

- 时间桶按 `COALESCE(ended_at, started_at)` 的本地日期分组，每组保留最新一个。
- `--keep-within` 内的全部保留，其余规则在窗口外生效。
- 默认策略：`--keep-within 14d --keep-daily 14 --keep-weekly 8 --keep-monthly 12 --keep-yearly 3 --keep-last 20`。
- 规则只产生候选；底线在候选之上再过滤一次。

### 恢复

```bash
sivtr storage restore codex/abc123
sivtr storage whereis codex/abc123     # 列出副本位置与最后校验时间
```

恢复流程：确认对象存在 → 重算哈希 → 解压 → 写入热缓存并设租约 → 清除墓碑（如果存在）。对象缺失时，错误信息列出查过的位置，对应 `git-annex whereis` 的输出。租约到期后热缓存副本可被 LRU 淘汰，冷层副本保持不动。

---

## 去冗余

| 冗余 | 处理 | 预期效果 |
| --- | --- | --- |
| `sets/*.json` 全量正文 | 只存 `WorkRef` 列表 + 编辑覆盖层（标题、锚点、顺序） | 205.9 MiB → 约 1 MiB |
| `records.blob_light` | `records` 元数据列直接提供 Light 视图 | 去掉一份每条记录的编码 |
| 终端记录的原文 + `*_ansi` | ANSI 存为 SGR span 区间，渲染时叠加 | 该段 1.84–1.88× 的重复消失 |
| 未压缩的 `blob` | zstd-19 冷对象 | 实测 gzip-6 3.27×，zstd 同级或更好 |
| 无界的 `listing-*.bin` | 上界 + archive 代数键 | 防止长期增长 |
| 可重建的 BM25 | 已有 8 个文件上界，保留 | 无变化 |

WorkSet 改为引用后，保存的集合自动成为保留底线：集合引用的会话不会被 `prune` 选中，正文从冷层读。这样“保存”既是选择也是保留，两个语义合一。

---

## 完整性

```bash
sivtr storage verify                 # 元数据：外键、孤儿对象、悬空引用
sivtr storage verify --read-data     # 重算每个对象的 SHA-256
sivtr storage verify --read-data-subset=10%
```

- 元数据检查很便宜，放进 `doctor`，每次运行都做。
- 数据检查默认抽样 10%，`--read-data` 全量；对应 `restic check --read-data` 与 `--read-data-subset`。
- 报告三类结果：`ok`、`mismatch`（哈希不符）、`missing`（元数据有引用、对象不存在）。
- 孤儿对象（无元数据引用）只报告，不自动删除；它们可能是中断写入的残留，也可能是 `meta.db` 损坏后的最后副本。

---

## 迁移与兼容

- `SCHEMA_VERSION` 从 7 升到 8 时，`session_stars` 在新表里先落地，再从 `sessions.starred = 1` 迁入星标，最后才 DROP 旧表。星标是用户意图，必须在任何重建之前保存。
- v7 → v8 只动星标，记录编码未变，因此原地升级、不重建，源文件已消失的会话也保留。更早的版本仍走有损重建，重建前先把星标迁入 `session_stars`。进入三层形态后，重建热缓存不再触碰元数据与冷层。
- 旧 `archive.db` 始终是派生状态，删除可以重建；源文件已消失且未加星的会话会随之消失，这一点在 CLI 与文档里明确写出。
- 远端协议抬版本，旧 peer 直接失败，不做混版本过滤。

---

## 分阶段

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0（本次） | `session_stars` 持久化星标；`remove_missing_sessions` 跳过星标会话；schema v8 迁移 | 星标在 schema 重建与源文件消失后都还在 |
| P1 | 冷层对象 + `meta.db` + `forget` / `prune` / `restore` / `verify`；`archive.db` 降级为热缓存 | 删掉 `cache/` 后所有内容仍可读 |
| P2 | 保留策略、墓碑宽限期、副本计数 | `prune` 在副本不足时拒绝并说明缺口 |
| P3 | WorkSet 改引用 + 覆盖层；删 `blob_light`；ANSI span；缓存上界 | 同一段文本在 home 内只有一份 |
| P4 | 远端副本计入 `copies`；对象加密；墓碑跨设备同步 | 两台设备之间 forget / restore 可往返 |

P0 只做两件事：把星标搬出派生表，让重建不再清空用户意图；让删除源文件后的清理尊重星标。`retained` 这类“内容已被冷层保存”的系统状态属于 P1，它由对象是否存在推导，不单独存一个用户可设的开关，避免和星标重复。

---

## 不在范围

- 内容定义分块去重（CDC）与跨会话块级去重。
- 纠删码、多副本一致性协议。
- 合规级 WORM / legal hold 产品面。
- 云端服务商选型与计费模型。冷层是同步单位，传输复用现有 iroh / share / grant / mount。
- 删除 provider 自己的原生日志。

---

## 风险

- **`prune` 不可逆。** 所有破坏性路径默认 dry-run，执行需要 `--apply`；宽限期、副本计数、底线规则三层拦截。
- **内容哈希泄露相等性。** 相同文本产生相同哈希，跨会话可见。本地单用户场景可接受；引入远端或加密时改为带密钥的 HMAC，哈希只在本 home 内可比。
- **`meta.db` 成为新的单点。** 它体积小，可以随 `sivtr export` 一起备份；`verify` 能发现损坏，冷层对象与 WorkSet 引用可以部分重建元数据。
- **热窗口过小导致读放大。** 默认 30 天 / 256 MiB 起步，按实际命中率调整，`storage status` 报告冷读次数。

---

## 参考资料

- restic: [Removing backup snapshots](https://restic.readthedocs.io/en/stable/060_forget.html)、[Checking integrity](https://restic.readthedocs.io/en/stable/045_working_with_repos.html#checking-integrity)
- borg: [borg prune](https://borgbackup.readthedocs.io/en/stable/usage/prune.html)、[borg compact](https://borgbackup.readthedocs.io/en/stable/usage/compact.html)
- git: [`git-reflog`](https://git-scm.com/docs/git-reflog)、[`git-gc`](https://git-scm.com/docs/git-gc)
- git-annex: [`numcopies`](https://git-annex.branchable.com/copies/)、[`git annex drop`](https://git-annex.branchable.com/git-annex-drop/)
- S3: [Lifecycle transitions](https://docs.aws.amazon.com/AmazonS3/latest/userguide/lifecycle-transition-general-considerations.html)、[Restoring archived objects](https://docs.aws.amazon.com/AmazonS3/latest/userguide/restoring-objects.html)、[Object Lock](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html)
- SQLite: [`VACUUM`](https://www.sqlite.org/lang_vacuum.html)、[Write-ahead logging](https://www.sqlite.org/wal.html)
