# 优化知识库检索效果

## Goal

把 Kivio 知识库检索从「黑盒、中文自然语言问题经常搜不到、无法判断哪一步坏了」改造成**可测量、可解释、可调优**的检索链路。不换本地 SQLite/sqlite-vec 存储架构，不引入 LLM 依赖的检索步骤（query rewrite、contextual chunk 等延后为独立任务）。

按 2026 业界共识（详见 `research/rag-retrieval-2026-refresh.md`）：RAG 失败约 73% 出在检索层；修复顺序是先建评测、修词法召回、分离参数、加阈值，再谈高级技术。本任务只做**确定性、可归因**的核心修复。

## 交付物（7 件，按依赖排序）

### D1. 检索测试台（Retrieval Test）

现状：检索是黑盒，答不好无法定位是解析/分块/向量/关键词/融合/重排哪一步的问题。

要求：

- 知识库设置内提供独立于聊天的检索测试入口：选库（一或多个）、输入查询、运行。
- 分阶段展示：向量通道命中+分数+耗时、关键词通道命中+耗时、融合排名、重排前后对比+耗时、阈值/去重淘汰原因、最终进入上下文的条目。
- **必须调用与 `knowledge_search` 完全相同的检索核心**，不得复制一套测试专用实现。
- 展示 rerank 降级状态（失败时明确显示 fallback，不静默）。

### D2. 修复中文关键词召回

现状：`store.rs` 把整句查询包成 FTS5 phrase（`"..."`），trigram 分词下等于要求文档**连续包含**用户问句原文，中文自然语言问题基本零命中（已本地复现）。

要求：

- 自然语言问题默认拆为安全的 term OR/AND 查询；不再整句 phrase。
- 保留 exact-match 能力：错误码、版本号、编号、引号内短语走精确通道。
- FTS 可检索正文 + 文档名 + 标题路径（schema 扩展 + 重建索引迁移）。
- 查询构造为纯函数，安全处理引号、FTS 操作符、标点、空查询、中英混合；有对抗性测试。
- trigram 的 <3 字符盲区（单字、`AI` 等）需有明确处理策略（跳过该 term 或降级向量通道），不得报错。
- **分层升级路径**：本任务做 term-OR（零依赖）；若评测显示仍不足，同 Phase 内评估 bigram 预处理（索引约 2×）；jieba/libsimple/自建 BM25 为后续独立任务，本任务不引入原生扩展。

### D3. Embedding 查询/文档角色分离

现状：入库和查询用同一个 `{model,input}` 请求体，没有 query/document 非对称（Voyage `input_type`、Jina task、E5/BGE prefix 全部缺失）。

要求：

- 引入 `EmbeddingRetrievalProfile`：query/document 各自的 input_type、prefix、受限 extra body。
- 为已知模型族（Voyage、Jina、E5、BGE、Qwen3-Embedding）提供预设；留空 = 现有对称行为（完全向后兼容）。
- **document 侧表示变更（prefix/input_type/model/dim）强制要求重建索引**；query-only 变更可热切。禁止静默出现新旧向量语义混用。
- 日志记录生效的角色配置便于诊断；不得记录 API Key 或完整私有文档。

### D4. 分离候选池参数

现状：最终 topK=5，rerank 候选 `max(topK*4,20)`，召回/重排/上下文数量耦合不可独立调。

要求：

- 拆为 `candidateK`（召回池）、`rerankTopK`（送重排数）、`contextTopK`（最终给模型数），各有上下界与 serde default。
- 业界基准形状为「召回 20–50（大池可到 150）→ 重排 → 送 3–8」；**默认值由 D7 评测确定，不照抄经验数字**。
- 多库检索：按库召回有限候选后全局融合，总候选受硬上限约束，防止库数量线性放大成本。
- 各阶段原始分数（vector distance、BM25 rank、fusion score、rerank score）保留到诊断层，不在 store 层丢弃。

### D5. 真正的相关性阈值

现状：过滤条件是 RRF score > 0，而 RRF 命中分数天然为正 —— 等于没有阈值，低相关内容照样填满 TopK。根因：纯名次融合丢弃分数量级，本身不可做阈值。

要求：

- rerank 开启时：用 rerank relevance score 做阈值（业界现行做法；calibrated 分数可直接设固定值）。
- rerank 关闭时：用**归一化融合分数**或 vector distance 上限做明确定义的 cutoff，禁止拿裸 RRF 分数当相似度。
- 阈值配置携带 score kind，换算法/模型后不得沿用错误量纲。
- 无结果过阈值时返回明确的「知识库未找到相关内容」，不得用低相关结果凑数；负样本测试覆盖。
- 默认阈值由 D7 评测集确定或采用保守兼容值（宁可放行也不误杀），不凭空写死。

### D6. 去重复片段

现状：同文档相邻/高重叠 chunk 可占满全部上下文槽位。

要求：

- 确定性规则去重：同 doc 相邻 chunk 合并或保留高分者；文本归一化后按包含关系/n-gram 重叠判定。
- **去重发生在送重排之前**（业界模式：先多样化再重排，省重排成本且提升结果多样性）。
- 去重不得删除唯一含答案的片段；单测覆盖「重复窗口」「唯一答案窗口」「多文档多样性」。
- 淘汰决策（kept/duplicate/below_threshold/truncated）进入诊断，D1 测试台可见。
- 第一阶段不引入 MMR/额外 embedding 调用。

### D7. 最小评测集

现状：没有任何可重复的检索质量度量，改了不知好坏。

要求：

- 仓库内人工构造的中英文测试语料 + 查询 + 相关性标注（不含任何用户私有内容/凭据），覆盖：中文自然语言改写、英文、混合语言、错误码/编号、无答案负样本。
- 评测 runner 直接调用检索核心；embedding 用固定向量 fixture/mock，另留可选 live smoke test。
- 指标：Recall@5/10/20、MRR、nDCG@10、负样本误召回率。
- 修改前先跑 baseline 存档；每项检索改动可用同一命令对比前后。

## 贯穿红线（不单列交付物）

- **兼容**：现有 `libraries.json`、`store.db`、source snapshot 必须可迁移或继续读取；FTS schema 变更走 version 检测 + 重建（chunk 文本已在库，无需重新解析文档）；旧配置有安全默认值。
- **隐私**：日志/评测不出现 API Key 或完整私有文档内容。
- **回滚**：保留 legacy 词法查询 feature flag 直到评测通过；不删除 source snapshots。

## Acceptance Criteria

- [ ] AC1：Retrieval Test 使用生产检索核心，展示 vector/keyword/fusion/rerank(含 fallback)/threshold/dedup 各阶段结果与耗时。
- [ ] AC2：中文自然语言问题无需与文档整句一致即可经关键词通道召回；错误码/编号精确命中仍有效；<3 字符 term 不报错。
- [ ] AC3：关键词索引覆盖正文、文档名、标题路径；中英混合、标点、FTS 特殊字符有对抗测试。
- [ ] AC4：至少一种 query/document 非对称 profile 有端到端（mock）测试；留空 profile 行为与现有完全一致；document 侧变更触发重建要求。
- [ ] AC5：candidateK/rerankTopK/contextTopK 独立生效且有上下界；rerank 失败在测试台可见。
- [ ] AC6：低于阈值的结果不进入最终上下文；负样本返回空结果而非无关 TopK；阈值带 score kind。
- [ ] AC7：高重叠/相邻重复不挤占全部槽位；唯一答案片段不被误删；去重在重排前执行。
- [ ] AC8：评测命令输出 Recall@5/10/20、MRR、nDCG@10 及负样本指标；改动前 baseline 已存档。
- [ ] AC9：旧知识库可继续使用或 UI 明确要求一次性重建；无静默向量语义不一致。
- [ ] AC10：Rust 测试、前端 lint/typecheck、相关 UI 测试通过。

## Out of scope（后续独立任务）

- LLM query rewrite / multi-query / HyDE / query decomposition。
- 父子 chunk、邻居扩展、contextual chunk generation（Anthropic 方案，数字见调研）。
- jieba/libsimple 原生分词扩展、自建 BM25（触发条件：D7 评测证明 term-OR + 向量仍不足）。
- 换向量数据库、GraphRAG、ColBERT/SPLADE/LTR、完整 Agentic Retrieval。
- 用户私有知识库作为评测数据。
