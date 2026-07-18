# RAG 检索调研（2026 刷新版）

> 目的：在 `rag-retrieval-research.md` 基础上刷新到 2026 最新业界做法，聚焦 Kivio 实际约束
> （本地 SQLite + sqlite-vec + FTS5 trigram、中文为主、provider-agnostic embedding、可选 rerank）。
> 结论：既有范围决策（先修可测量的核心检索，延后 query rewrite / 父子 chunk / contextual）依然成立，
> 但有两处需要写进 design：**中文 FTS 分词的具体升级路径** 和 **用 calibrated rerank 分数做阈值**。

## 0. 2026 业界共识（方向）

- 失败点在检索不在生成：多方分析称 RAG 失败约 **73% 出在 retrieval**；naive RAG（单向量库+top-k+塞 prompt）在真实企业语料上有约 **40% 检索失败率**，已不是任何严肃系统的参考架构。
- 最高杠杆的顺序：**先修 chunking → hybrid 检索 → 激进 rerank → 持续评测**。换更强的生成模型只是放大喂进去的质量，不解决检索问题。
- 长上下文没有杀死 RAG：百万 token 窗口下 naive RAG 死了，检索层工程反而更重要。

## 1. Hybrid 检索（已是生产标准，Kivio 已具备）

- BM25 关键词 + dense 向量并行，用 **RRF 融合**是默认方案；文档在两路都靠前会被加权。典型 over-fetch 到 top 20–50 再融合。
- 收益：hybrid 相比纯向量 **recall +最高 17%，额外延迟 <6ms**。
- BM25 与 dense 的失败模式互补：BM25 丢同义/改写，dense 丢稀有词、数字 ID、精确短语/编号。
- **融合方式的坑（对 Kivio R5 直接相关）**：纯 RRF 只用名次、**丢弃分数量级**，所以无法据此设阈值。Weaviate 提供 `relativeScoreFusion`（归一化分数融合）保留距离信息。Kivio 现在正是纯 RRF + `score>0`，这就是"没有有效阈值"的根因——名次融合分数天然为正。

来源: [callmissed](https://www.callmissed.com/en/blog/rag-best-practices-2026) · [ailearningguides](https://ailearningguides.com/rag-production-patterns-2026/) · [Weaviate hybrid](https://docs.weaviate.io/weaviate/concepts/search/hybrid-search) · [PatentLLM 200行 hybrid](https://media.patentllm.org/blog/database/hybrid-rag-200-lines)

## 2. 中文 FTS 分词（Kivio 的 1 号病灶，需在 design 里定路径）

现状：FTS5 `trigram` + 把整句包成 `"..."` phrase 查询。trigram 是**无依赖的应急方案**，对精确子串/编号 OK，但**改写/同义/词序不同就退化**，且 trigram 要求匹配 token **≥3 字符**（单字、`AI`/`os` 这类 1–2 字符查询根本命不中）。

业界对 SQLite 中文 FTS 的四条路径（从轻到重）：

1. **修查询构造（最省，Kivio Phase B 已计划）**：phrase → 安全的 term `OR`/`AND`，保留 exact-phrase 作为可选通道。这一步就能把"零命中"问题解决大半，零新依赖。
2. **bigram 预处理**：入库/查询前把中文切成 2-gram 再进 FTS。7 行代码、无 jieba/无外部扩展，对中文改写召回明显好于 trigram phrase；代价是索引约 **2× 膨胀**，且对韩日效果不如中文。要点：**mangled 的分词文本只用于索引，LLM/用户看的原文另存**（join rowid 取回）。
3. **libsimple（`wangfenjin/simple`，MIT）**：专用 FTS5 中文扩展，含 `simple`（字符级，解决多音字）与 **jieba** 分词，支持**拼音**；有 macOS/Windows/预编译二进制 + Rust 集成示例。加载 <2ms，首次 jieba 加载词典约 4s。代价：引入一个 **per-platform 原生 C++ 扩展**（要打包进 DMG/MSI，跨平台构建成本）。
4. **绕过 FTS5 自建 jieba + BM25**：jieba 分词 + 停用词表（如 HIT 停用词）+ 手写 BM25。最灵活也最重，通常用于纯 Chinese-heavy 生产 RAG。

对 Kivio 的取舍（ponytail）：**先做 1（Phase B 计划内），把 2（bigram）作为同一 Phase 内的低成本增强候选**；**3/4 列为后续独立任务**——只有在评测证明 term-OR + 向量通道仍不够时才值得引入原生扩展/自建 BM25 的打包与维护成本。

来源: [wangfenjin/simple](https://github.com/wangfenjin/simple) · [bigram 7行修复](https://dev.to/foxck016077/sqlite-fts5-wont-tokenize-chinese-heres-the-7-line-bigram-fix-that-did-4fcc) · [chroma trigram CJK bug](https://github.com/chroma-core/chroma/issues/1073) · [EasyRAG jieba+BM25](https://arxiv.org/pdf/2410.10315)

## 3. Embedding 查询/文档非对称（Kivio 的 3 号病灶）

- 明确的业界模式：**query 侧加 instruction/prefix，document 侧通常不加**（E5、GTE-Qwen2、NV-Embed、Qwen3-Embedding 都遵循）。API 层各家不同：
  - Voyage：`input_type="query"` / `"document"`
  - Jina v3：task-specific（retrieval.query / retrieval.passage）
  - E5 / BGE：query 前缀 `query:` / `Represent this sentence...`，passage 前缀 `passage:`
  - **BGE-M3、Jina-v3 这类不强制 instruction**（直接编码即可），所以配置模型必须"可选、可留空 = legacy 对称行为"。
- 2026 主力模型：**Voyage-3**（MTEB 67.1，32k 上下文，检索最强档）、**Jina-v3**（65.5，长文档友好）、**Qwen3-Embedding**（多语，0.6B/8B）、**BGE-M3**（开源，dense+sparse+multi-vector 一体，1000+ 语言，8192 token，hybrid 默认盘）。
- **MRL（Matryoshka）**：Voyage/Jina v4 训练即支持维度截断——降维好不好取决于是否为此训练，而非模型大小。（与 Kivio "改 dim 要重建索引"的约束一致。）

对 Kivio：R3 的 `EmbeddingRetrievalProfile` 设计正确。**关键约束**——改变 document 侧表示（prefix/input_type/task/model/dim）必须触发**重建索引**；query 侧可安全热切。legacy provider 默认对称、留空即旧行为。

来源: [Milvus 选型 2026](https://milvus.io/blog/choose-embedding-model-rag-2026.md) · [ailog embeddings 2026](https://app.ailog.fr/en/blog/news/embedding-models-2026) · [SitEmb 指令编码](https://arxiv.org/pdf/2508.01959)

## 4. Rerank + 阈值（Kivio 的 4/5 号病灶）

- 标准管线：`bi-encoder 取 top 20–50 → cross-encoder rerank → 3–5 条进 LLM`。经验法则：**取 20，重排到 5，送 3–5**；重排 100+ 候选很少划算（signal 在头部）。Contextual Retrieval 实测用了 **150 候选 → rerank → 20** 的更大池。
- **阈值的关键洞察**：cross-encoder 的 **relevance 分数比融合名次更适合做阈值**。**calibrated 分数**（如 ZeroEntropy zerank-2）可直接 `score > 0.7` 过滤；未校准的分数需在自己语料上先看分布再定阈值（Cohere 也这么建议）。→ **Kivio R5 应优先用 rerank relevance 分数做阈值**；无 rerank 时退回"归一化融合分数 + 保守阈值"，别用裸 RRF。
- 2026 reranker 选项：Cohere Rerank 3（闭源、快、多语）、**BGE-reranker-v2-m3**（Apache-2.0、100+ 语言、GPU 50–100ms，自托管首选）、Jina v3（listwise 0.6B）、Qwen3-Reranker（yes/no logit，准但慢 >1s）、ZeroEntropy zerank-2（无 GPU、便宜、**calibrated 分数**、100+ 语言）。
- 校准/验证警告：**ARAGOG 发现某些语料上 Cohere Rerank 相比 naive 无明显优势**——rerank 必须在自己语料上验证，不能默认开就更好。rerank 前先按来源/段落**去重多样化**再送重排。

对 Kivio：R4 拆 `candidateK / rerankTopK / contextTopK` 与业界一致；默认值应由评测集定，别照抄草案数字。rerank 失败要**显式 fallback 状态**（测试页可见），别静默。

来源: [futureagi rerankers 2026](https://futureagi.com/blog/best-rerankers-for-rag-2026/) · [ZeroEntropy 指南](https://zeroentropy.dev/articles/ultimate-guide-to-choosing-the-best-reranking-model-in-2025/) · [BSWEN 开源vsAPI](https://docs.bswen.com/blog/2026-02-25-best-reranker-models/) · [Cohere rerank best practices](https://docs.cohere.com/docs/reranking-best-practices)

## 5. Chunking / Contextual（**确认延后**，但记录目标态）

- 2026 最高杠杆其实是 chunking：semantic chunking（相邻句 cosine 掉到阈值就断）据称比固定长度 **+最高 70% 检索准确率**；structure-aware（按标题/函数切）；**late chunking**（先整篇编码再切）；每 chunk 带 metadata（source/heading/page/parent id）。
- **Contextual Retrieval（Anthropic）具体数字**（top-20 失败率基线 5.7%）：
  - contextual embeddings：**5.7% → 3.7%**（-35%）
  - + contextual BM25：**→ 2.9%**（-49%）
  - + rerank：**→ 1.9%**（-67%）
  - 方法：每 chunk 前置 50–100 token 的 LLM 生成情境说明（Haiku + prompt caching，约 **$1.02/百万文档 token**）。
- **为何 Kivio 仍延后**：这些是 LLM 依赖 + 逐 chunk 成本 + 高变量改造，叠加会让效果无法归因。先把可测量核心修好、建立评测基线，再单独上 contextual / 父子 chunk，每项独立对比 baseline。→ **范围决策不变**，但 design 应把 heading-path 纳入 FTS（低成本、属 structure-aware 的一部分，Phase B 已含）。

来源: [Anthropic Contextual Retrieval](https://www.anthropic.com/engineering/contextual-retrieval) · [atlan 12 advanced RAG](https://atlan.com/know/advanced-rag-techniques/)

## 6. 评测（2026 已是硬要求，对应 Kivio R7）

- RAGAS 式评测 + 全链路可观测被列为生产架构核心组件（72% 企业已上生产 RAG）。
- Kivio 的 Recall@5/10/20 + MRR + 负样本方向正确；建议补 **nDCG@10**（业界横评主指标）。测试集小而标注清晰即可，含"库内无答案"负样本以验证阈值真的能返回空。

## 7. 对本任务的净结论

范围**不变**（先修可测核心；rewrite / 父子 chunk / contextual / 换向量库延后）。需要写进 `design.md` 的三点更新：

1. **中文 FTS 升级路径分层**：Phase B 先修 phrase→term-OR（零依赖）；bigram 预处理作同 Phase 低成本增强候选；libsimple(jieba/pinyin) 与自建 BM25 列为后续独立任务的触发条件（评测证明词法召回仍不足时）。
2. **阈值改用 calibrated rerank 分数**：R5 优先 rerank relevance 分数阈值；无 rerank 时用**归一化融合分数**（非裸 RRF）+ 保守阈值，默认值由评测集定。
3. **Embedding profile 的重建触发**：document 侧表示变更强制重建索引，query 侧热切；legacy 留空 = 旧对称行为。

次要：FTS 纳入 heading-path/doc-name（Phase B 已计划，属 structure-aware 低成本项）；评测补 nDCG@10；rerank 前按来源去重再送（与 R6 去重协同）。
