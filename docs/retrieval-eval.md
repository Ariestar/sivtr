# Retrieval Evaluation

`sivtr eval` 用**真实数据快照**度量检索质量：冻结语料 + 人工标注查询（qrels）。
评估排序质量：每个查询把整个语料按策略排序，取 top-k，算标准 IR 指标
（recall@k / precision@k / MRR / NDCG@k，二元相关，k=5）。

## 工作流

```bash
sivtr eval --create-snapshot snap.json   # dump 当前 workspace 真实记录（queries 为空）
# 编辑 snap.json：加标注查询 { name, query, field, relevant: [refs] }
sivtr eval --snapshot snap.json --sort newest      # baseline
sivtr eval --snapshot snap.json --sort relevance   # BM25 排序
sivtr eval --snapshot snap.json --json             # 机器可读
sivtr eval --snapshot snap.json --export dir       # trec_eval 格式 (qrels.txt + results.txt)
```

快照含个人记录，已 gitignore，**永不提交**。

## 真实数据结果（2026-08-04）

快照：1328 条（442 terminal + 886 chat_turn，claude/pi/codex/grok/opencode/qoder，68% 含中文），22 个标注查询。

### `--sort newest`（recency baseline）

mean recall@5 0.008 / prec 0.036 / mrr 0.045 / ndcg@5 0.030 —— 基本全 0。

### `--sort relevance`（BM25 + CJK bigram，头尾双窗）

```
query                         relevant retrieved  recall@5    prec@5       mrr    ndcg@5
cargo install                       73         5     0.068     1.000     1.000     1.000
serve status                         2         5     1.000     0.400     1.000     1.000
share invite                         1         5     0.000     0.000     0.000     0.000
remote list                          8         5     0.625     1.000     1.000     1.000
doctor                               9         5     0.444     0.800     0.500     0.661
mcp uninstall                       13         5     0.154     0.400     1.000     0.553
connection refused                   4         5     0.750     0.600     1.000     0.832
mcp cmd                              5         5     0.400     0.400     0.500     0.345
title cargo install                 73         5     0.068     1.000     1.000     1.000
session qoder                       14         5     0.214     0.600     1.000     0.684
zh 重构                               90         5     0.056     1.000     1.000     1.000
zh 工具调用                             20         5     0.250     1.000     1.000     1.000
zh 性能                               25         5     0.200     1.000     1.000     1.000
panicked                            37         5     0.135     1.000     1.000     1.000
kubectl                              5         5     0.200     0.200     1.000     0.339
provider grok                       19         5     0.211     0.800     1.000     0.830
rust compile error                  56         5     0.018     0.200     1.000     0.339
command not found                   24         5     0.000     0.000     0.000     0.000
docker                              33         5     0.152     1.000     1.000     1.000
npm                                 43         5     0.116     1.000     1.000     1.000
permission denied                    7         5     0.429     0.600     1.000     0.723
zh compile error                     7         5     0.571     0.800     1.000     0.830
mean                                                 0.276     0.673     0.864     0.734
```

vs baseline：recall@5 ×35、ndcg@5 ×24。

### 排序策略演进（同一 22 查询快照实测）

| 策略 | recall@5 | prec@5 | mrr | ndcg@5 | 说明 |
| --- | --- | --- | --- | --- | --- |
| 无截断（全量索引） | 0.205 | 0.509 | 0.623 | 0.543 | 长对话里高频常见词（`share`/`status` df 300+）淹没命令记录 |
| 800 token 头截断（8/2 实现） | 0.276 | 0.600 | 0.920 | 0.709 | 命令查询最好，但长对话尾部错误文本被整体丢弃 |
| 头尾双窗 800（8/4） | 0.276 | 0.673 | 0.864 | 0.734 | 错误查询恢复，命令查询小幅回落 |
| **字段加权 + BM25+ + 门控 PRF（af1d613）** | **0.365** | **0.736** | **0.924** | **0.828** | 见 `docs/retrieval-literature.md`：title 字段加权（多 token 查询）、BM25+ δ 下界、PRF 按查询难度门控 |
| **Passage retrieval over typed WorkParts（当前）** | **0.403** | **0.773** | **0.964** | **0.861** | 索引单位 = 每个类型化 WorkPart（Callan SIGIR'94 最佳匹配段落），按 part 种类做长度归一化，记录分 = 各 part 最大分；替换头尾双窗。内容类 kind 权重统一 1.0，Title/Command 3.0 按查询 token 数门控，k1=2.0 |

### 2026-08-04 最终配置（passage retrieval over typed WorkParts）

```
query                         relevant retrieved  recall@5    prec@5       mrr    ndcg@5
cargo install                       73         5     0.068     1.000     1.000     1.000
serve status                         2         5     1.000     0.400     1.000     1.000
share invite                         1         5     1.000     0.200     1.000     1.000
remote list                          8         5     0.625     1.000     1.000     1.000
doctor                               9         5     0.556     1.000     1.000     1.000
mcp uninstall                       13         5     0.154     0.400     1.000     0.553
connection refused                   4         5     1.000     0.800     1.000     0.956
mcp cmd                              5         5     1.000     1.000     1.000     1.000
title cargo install                 73         5     0.068     1.000     1.000     1.000
session qoder                       14         5     0.214     0.600     1.000     0.723
zh 重构                               90         5     0.056     1.000     1.000     1.000
zh 工具调用                             20         5     0.250     1.000     1.000     1.000
zh 性能                               25         5     0.200     1.000     1.000     1.000
panicked                            37         5     0.135     1.000     1.000     1.000
kubectl                              5         5     1.000     1.000     1.000     1.000
provider grok                       19         5     0.105     0.400     1.000     0.470
rust compile error                  56         5     0.018     0.200     0.200     0.131
command not found                   24         5     0.208     1.000     1.000     1.000
docker                              33         5     0.091     0.600     1.000     0.616
npm                                 43         5     0.116     1.000     1.000     1.000
permission denied                    7         5     0.286     0.400     1.000     0.485
zh compile error                     7         5     0.714     1.000     1.000     1.000
mean                                                 0.403     0.773     0.964     0.861
```

（与 `{SCRATCH}/eval-after.txt` 捕获一致，两次运行完全相同。）

## 关键改进

- **CJK bigram 分词**：中文此前完全不可用（`is_alphanumeric` 把整段中文当一个 token）。改为重叠二元组（Lucene CJKAnalyzer 风格，`重构逻辑` → `重构/构逻/逻辑`）后，3 个中文查询 prec@5 / mrr / ndcg@5 全 1.0。零依赖，对所有中文用户生效（本语料 68% 中文）。
- **标准 BM25（k1=1.2, b=0.75）+ 头尾双窗**：替换 bm25 crate 的裸 idf×tf 后，命令史查询从 0 恢复（`serve status`/`remote list` 等 mrr 1.0）。头窗丢弃长对话中段的高频常见词，尾窗保住会话尾部错误文本。
- **字段加权 + BM25+ + 门控 PRF（2026-08-04，见 `docs/retrieval-literature.md`）**：title 作为独立字段（多 token 查询权重 3，单 token 查询权重 0——避免 `grok` 类内容查询被同名终端命令淹没）；BM25+ δ 下界让「命中查询的长文档」严格高于「不匹配的短文档」；PRF 伪相关反馈只对稀有词查询生效（df 门控），`share invite`/`doctor`/`mcp cmd` 从 0.3-0.7 拉到 1.0。均值 recall@5 0.276→0.365、ndcg@5 0.734→0.828。
- **Passage retrieval over typed WorkParts（2026-08-04 晚，当前）**：不再对整条记录做头尾截窗——索引单位改为每个类型化 WorkPart（Prompt/Command/Output/Error/ToolResult/Assistant/User/Thinking），按 part 种类分别做长度归一化（每类独立 avgdl），记录得分 = 各 part 得分的最大值（Callan SIGIR'94 最佳匹配段落）。这正是适配 sivtr 内容结构的「优雅方案」：对话中段不再是「被丢弃」或「被稀释」，而是若干有边界的短段落；错误文本天然落在自己的 part 里。标题/命令 passage 权重 3.0（多 token 查询），内容类统一 1.0（>1 会让终端短输出记录淹没内容查询，实测验证），k1 2.0。均值 recall@5 0.365→0.403、ndcg@5 0.828→0.861；`kubectl` 0.339→1.0、`command not found` 0→1.0、`connection refused` 0.832→0.956。

## 诚实结论

- **错误/中文/工具查询大幅有效**：`connection refused`、`panicked`、中文查询、`docker`/`npm` top-5 基本全相关。
- **命令史查询用字段加权后恢复**：`share invite`/`doctor`/`mcp cmd` 在内容 BM25 + title 字段加权下全部 mrr 1.0，不再依赖尾窗副作用。
- **「文档中段」问题已被 passage 模型结构性解决**：`kubectl`、`command not found` 这两个曾因词在头尾窗之外而完全失分的查询现在都是 1.0——不是靠更大的窗口，而是因为错误文本本来就住在自己的 WorkPart 里，段落检索天然命中。剩余弱查询（`rust compile error` 0.131、`docker` 0.616、`provider grok` 0.470、`permission denied` 0.485）不再是没有索引，而是 top-5 精确率问题：`rust compile error` 有 56 条相关记录，recall@5 上限本身只有 0.09；内容类查询的候选池大、相关密度低，k=5 窗口下 ndcg 被稀释。这些留给 `--in output` 精确匹配 / 语义层（roadmap 延后项）。
- **recency/failure 加分不做**（已砍）：数据集特定启发，不够泛化。RRF 融合中 recency 信号因此移除，实测反而降低 ndcg（见文献文档）。

## 已知限制

- 无停用词/词干；Robertson idf 恒非负（不再有负 idf 问题）。
- 快照标注人工判断，相关集不完整（如 `share invite` 只标了无参数形式）。
- 头尾窗 800 是启发式常量：窗口越小错误召回越差，越大命令查询越容易被长对话中段词淹没。
- 性能：BM25 索引每个 Searcher 惰性构建一次（O(语料)）；长对话索引被窗口截断在 ≤1600 token，构建开销有界。

## 交叉验证

`--export` 输出标准 trec_eval 格式，可用 `trec_eval` / `ir_measures` / `pytrec_eval` 核对。
