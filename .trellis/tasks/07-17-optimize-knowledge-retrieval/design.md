# Technical Design — 优化知识库检索效果

## 1. Design objective

在不替换本地 SQLite/sqlite-vec 存储的前提下，将知识库检索改造成单一可复用的多阶段管线，并让聊天工具与 Retrieval Test 共用该管线。

```text
RetrievalRequest
  → query preparation
  → dense recall + lexical recall
  → fusion
  → deduplication
  → optional rerank
  → threshold
  → context selection
  → RetrievalResponse + diagnostics
```

## 2. Boundaries

### Backend core

- `src-tauri/src/chat/knowledge_base/embeddings.rs`
  - provider/model-aware query/document request construction。
- `src-tauri/src/chat/knowledge_base/store.rs`
  - vector、FTS、字段检索、原始分数和融合。
- `src-tauri/src/chat/knowledge_base/mod.rs`
  - 多库检索包装和 schema/迁移边界。
- 新建或拆分 retrieval service module
  - 统一编排候选召回、去重、rerank、阈值和诊断。
- `src-tauri/src/mcp/native_registry.rs`
  - `knowledge_search` 仅做参数解析与结果格式化，调用统一 retrieval service。
- 新 Tauri command
  - Retrieval Test 调用统一 retrieval service。

### Frontend

- `src/settings/KnowledgeRagPanel.tsx`
  - 参数设置入口；不直接复制后端检索逻辑。
- 新 Retrieval Test 视图/组件
  - 查询输入、知识库选择、阶段结果、耗时和 fallback 展示。
- `src/api/tauri.ts` 或知识库 API 模块
  - request/response/diagnostics 类型镜像。

## 3. Retrieval contracts

建议引入稳定的内部契约：

```text
RetrievalRequest
- query
- kb_ids
- candidate_k
- rerank_top_k
- context_top_k
- diagnostics

RetrievalCandidate
- kb_id / doc_id / chunk_id
- text / doc_name / heading_path / order_index
- vector_rank / vector_distance / vector_score
- keyword_rank / keyword_score
- fused_score
- rerank_score?
- final_rank?
- decision: kept | duplicate | below_threshold | truncated

RetrievalResponse
- hits
- candidates?        # diagnostics=true 时返回
- timings
- rerank_status
- effective_config
```

生产聊天默认不需要携带所有候选正文诊断；测试命令可以返回完整诊断。

## 4. Lexical retrieval design

### Current defect

整条 query 被作为 quoted phrase，导致自然语言问题只有在原文出现连续相同子串时才命中。

### Proposed strategy

第一阶段采用确定性、无需额外 LLM 的 query builder：

1. 规范化空白和标点。
2. 提取明显 exact terms：错误码、版本号、URL、编号、带连接符标识符、引号内短语。
3. 对普通词项构建安全 OR/AND 形式；查询过长时限制 term 数。
4. exact phrase 作为额外通道或 boost，而不是唯一查询。
5. FTS 表扩展为正文、文档名、标题路径字段，或构造单独 searchable text 字段；迁移通过重建 FTS 索引完成。

FTS 查询构造必须由纯函数完成并有 adversarial tests，禁止直接拼接未经转义的 FTS 运算符。

## 5. Fusion design

保留 Weighted RRF 作为兼容模式，同时为后续评测预留 Relative Score Fusion：

- RRF：适合不同分数量纲、稳定，但丢失分数间距。
- Relative score：分别归一化 vector 与 BM25 分数，再按权重线性组合；数据较少或分数相同时需定义退化行为。

第一轮实现是否切换默认算法由评测结果决定，而不是直接改变默认。

## 6. Embedding role design

为知识库绑定的 Embedding 增加可序列化 profile：

```text
EmbeddingRetrievalProfile
- query_input_type?
- document_input_type?
- query_prefix?
- document_prefix?
- extra_query_body?
- extra_document_body?
- normalize? / dimensions?（仅在 provider 支持时）
```

优先提供已知模型预设；高级自定义必须避免任意覆盖 `model`、`input`、鉴权等核心字段。

如果 profile 改变文档表示，则标记 library 需要重建；仅改变 query-only 参数时可立即生效，但必须保证与原索引模型契约兼容。

## 7. Candidate stages and defaults

初始建议边界，最终默认值由评测确定：

- candidateK: 20..200，建议起始 60。
- rerankTopK: 5..50，建议起始 12。
- contextTopK: 1..20，建议起始 8。

多库检索需要明确 candidate budget 是“每库”还是“全局”。推荐按库召回有限候选后全局融合，最终总候选受硬上限约束，防止挂载库数量线性放大网络成本。

## 8. Deduplication

第一阶段使用确定性规则：

- 同一 doc 中相邻且高度重叠的 Chunk 合并或只保留较高分者。
- 文本归一化后计算包含关系/字符 n-gram 重叠。
- 保留 decision diagnostics，Retrieval Test 可显示为什么被过滤。

不在第一阶段引入额外 Embedding MMR 调用。

## 9. Threshold semantics

- rerank 开启：使用 rerank provider 返回的 relevance score，阈值需按模型/profile 保存。
- rerank 关闭：不得把未归一化 RRF 分数包装成通用 0..1 相似度。可采用 vector distance 上限与关键词命中规则，或使用明确命名的 fusion cutoff。
- 阈值配置必须带 score kind，避免切换算法/模型后沿用错误量纲。

## 10. Evaluation and observability

- 测试 fixture 使用仓库内人工构造的中英文文档与标注 query。
- 评测 runner 直接调用纯存储/检索核心，网络 Embedding 使用固定向量 fixture 或 mock provider；另保留可选 live model smoke test。
- 指标：Recall@5/10/20、MRR、无答案误召回率；后续可加 nDCG 和 answer-level faithfulness。
- 生产日志记录阶段耗时、候选数量、模型/profile 名称和错误类型，不记录 API Key、完整 Query/Chunk（除非显式 debug 且经过隐私设计）。

## 11. Compatibility and migration

- `KnowledgeBaseConfig` 新字段全部有 serde default。
- FTS schema 变化通过 schema version 检测与重建处理；原始 Chunk 和 vector 行尽量不重算。
- Embedding document profile 变化必须重建向量。
- 旧库默认 profile 保持现有 `{model,input}` 行为，避免静默改变已有索引语义。

## 12. Rollback

- 保留 legacy lexical query/fusion feature flag 直到新评测和迁移稳定。
- schema 迁移前创建可重建标记；SQLite 中 Chunk 文本和 source snapshot 已存在，可重建 FTS。
- 新设置字段可回落到旧默认；不得删除原始 source snapshots。

## 13. Deferred follow-up branches

以下能力不进入本任务。完成核心检索评测后，以独立 Trellis 任务评估是否实施：

- LLM standalone-query rewrite / keyword extraction。
- multi-query、HyDE 和 query decomposition。
- parent-child chunks、邻居扩展和 contextual chunk generation。
- sparse embedding / ColBERT / knowledge graph。
