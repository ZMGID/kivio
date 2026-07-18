# 优化知识库检索效果

## Goal

将 Kivio 知识库检索从“已有 Hybrid/Rerank 组件但效果不可测、自然语言查询召回不稳定”的状态，改造成可测量、可解释、可调优的检索链路；优先解决已确认的关键词召回退化、Embedding 查询/文档角色未区分、无有效相关性阈值和候选池配置耦合问题。

用户价值：在不更换本地 SQLite 存储架构的前提下，提高正确知识片段进入最终上下文的概率，减少无关片段和重复片段，并让用户能够判断问题发生在解析、分块、召回、融合、重排还是回答阶段。

本任务采用“先修复并量化核心检索，再依据指标引入高级检索”的范围策略。核心交付包括 Retrieval Test、FTS 修复、Embedding query/document 角色、候选池分层、相关性阈值、去重和 Rerank 可观测性；不在本任务同时引入 LLM Query Rewrite 或父子 Chunk，避免多项高变量改造叠加后无法归因效果。

## Background and confirmed facts

- 当前知识库为每个库保存一个 `store.db`，使用 sqlite-vec `vec0` 做 cosine 向量检索，FTS5 trigram 做关键词检索，再用 RRF 融合；相关实现位于 `src-tauri/src/chat/knowledge_base/store.rs`。
- 当前 FTS 查询把完整自然语言查询包进双引号：`format!("\"{}\"", ...)`，使其成为 phrase 查询。自然语言问题与文档原句不一致时，关键词通道通常返回零结果；本地 SQLite trigram 验证已复现。
- 当前 Embedding 入库与查询均调用同一个 `embed_batch` 请求体 `{model,input}`，没有 provider/model-aware 的 query/document input type、task、prefix 或 extra body；见 `src-tauri/src/chat/knowledge_base/embeddings.rs`。
- 当前 `knowledge_search` 只使用一条传入 query；没有独立问题改写、关键词提取、多查询或问题拆解；见 `src-tauri/src/mcp/native_registry.rs`。
- 当前最终过滤仅为 RRF score `> 0`。RRF 命中分数天然为正，因此没有实际的“无相关结果”阈值。
- 当前默认最终 `topK=5`；启用 rerank 时候选池为 `max(topK*4, 20)`。候选召回数、重排输出数、最终上下文数没有独立配置。
- 当前 Markdown Chunk 会携带标题路径，普通文本/PDF/DOCX 仍是扁平 Chunk；没有父子 Chunk、邻居扩展、MMR 或结果去重。
- 当前设置页允许配置 Hybrid 权重、Rerank、Chunk tokens 和 TopK，但没有独立 Retrieval Test、通道级分数或 Recall@K 评测能力。
- 外部调研显示 RAGFlow、Dify、Pinecone、Weaviate、Qdrant、LlamaIndex 和 Anthropic 的成熟方案普遍采用：检索测试、较大候选池、相关性阈值、query/document 非对称 Embedding、查询改写、多路召回、重排和父子上下文扩展。

## Requirements

### R1. 检索必须可测试和可解释

- 提供独立于聊天回答的 Retrieval Test 能力，用户可选择知识库并输入查询。
- 结果至少展示最终排名、文档名、标题路径、片段文本和最终分数。
- 调试信息应能区分 vector、keyword、fusion 和 rerank 阶段的排名/分数及各阶段耗时。
- 检索测试必须调用与实际 `knowledge_search` 相同的核心管线，不能复制一套行为不同的测试实现。

### R2. 修复关键词召回

- 普通自然语言问题不得再默认作为完整 phrase 查询。
- 关键词通道应支持安全的关键词 OR/AND 查询，并保留精确短语、错误码、编号等 exact-match 能力。
- 文档名、标题路径和正文应可参与关键词检索；字段权重或融合策略必须可测试。
- FTS 查询必须安全处理引号、操作符、标点、空查询和中英文混合输入。

### R3. Embedding 必须支持检索任务角色

- 知识库必须能够区分 document/passage 和 query 的 Embedding 输入配置。
- 配置应能表达常见 provider/model 需要的 `input_type`、`task`、query/document prefix 或额外请求字段，同时保持现有 OpenAI-compatible provider 向后兼容。
- 同一个知识库入库和查询必须使用兼容的一对配置；改变文档侧表示的配置时必须要求重建索引。
- 运行时应记录实际生效的 Embedding 请求角色，便于诊断模型配置问题，但不得记录 API Key 或完整私有文档内容。

### R4. 分离召回、重排和上下文参数

- 分离 `candidateK`、`rerankTopK` 和 `contextTopK`。
- 没有 rerank 时仍应支持从较大候选池融合、去重后选择最终上下文。
- rerank 失败允许降级，但测试页和日志必须明确显示降级，不能只静默隐藏。
- 参数必须有安全边界，避免单次检索产生失控的网络请求、Token 使用或上下文体积。

### R5. 支持有效的相关性过滤

- 最终返回前必须能按可解释分数应用阈值。
- 如果启用 rerank，优先使用 rerank relevance score；没有 rerank 时使用经过定义和测试的 vector/fusion 判定。
- 没有结果达到阈值时必须返回明确的“知识库未找到相关内容”，不得用低相关 TopK 填满结果。
- 阈值默认值不能仅凭经验写死，必须由测试集结果或保守兼容策略确定。

### R6. 减少重复上下文

- 对高重叠 Chunk 和同文档相邻结果进行去重或合并，避免最终 TopK 被同一段内容占满。
- 去重不得删除唯一包含答案的片段；行为必须有单元测试覆盖。
- 第一阶段允许采用确定性的文本重叠/相邻 Chunk 策略，不强制引入 MMR 模型。

### R7. 建立最小检索评测契约

- 提供可版本控制的非私密测试语料和查询标注，用于验证关键词、向量、融合、阈值、去重和 rerank 管线。
- 至少计算 Recall@5、Recall@10、Recall@20、MRR，并包含“知识库无答案”的负样本。
- 关键检索改动必须能运行同一评测命令比较修改前后结果。
- 评测 fixture 不得包含用户真实私有知识库内容或凭据。

### R8. 兼容性和数据安全

- 现有 `libraries.json`、`store.db` 和源文件快照必须可迁移或继续读取。
- schema/index 变化必须有重建或迁移策略，不得静默产生新旧索引混用。
- 默认仍维持本地 SQLite/sqlite-vec 架构；本任务不以更换向量数据库作为前提。
- 设置迁移必须为旧配置提供安全默认值。

## Acceptance Criteria

- [ ] AC1：Retrieval Test 使用生产检索核心，能展示 vector、keyword、fusion、rerank/fallback 和最终结果。
- [ ] AC2：测试证明自然语言问题无需与文档整句一致即可通过关键词通道召回相关片段；精确编号/错误码仍可命中。
- [ ] AC3：关键词索引可检索正文、文档名和标题路径，并覆盖中文、英文、中英混合、标点和 FTS 特殊字符。
- [ ] AC4：至少一种需要 query/document 角色的 Embedding 配置具有端到端测试；旧 OpenAI-compatible 配置保持兼容。
- [ ] AC5：`candidateK`、`rerankTopK`、`contextTopK` 独立生效且有上下界；rerank 失败状态在测试结果中可见。
- [ ] AC6：低于阈值的结果不会进入最终上下文；负样本能够返回空结果而不是无关 TopK。
- [ ] AC7：高重叠或相邻重复结果不会挤占全部上下文槽位，且相关片段不会被错误去除。
- [ ] AC8：评测命令输出 Recall@5/10/20 和 MRR，并包含至少一组负样本指标。
- [ ] AC9：旧知识库数据可以继续使用，或 UI 明确要求/执行一次性重建；不存在静默的向量角色不一致。
- [ ] AC10：Rust 单元/集成测试、前端类型检查和相关 UI 测试通过。

## Out of scope

- 更换 sqlite-vec 为独立向量数据库。
- GraphRAG、知识图谱实体关系召回。
- ColBERT late interaction、SPLADE 或学习排序 LTR。
- 完整 Agentic Retrieval 和无限多轮搜索。
- LLM standalone-query rewrite、multi-query、HyDE 和 query decomposition。
- 父子 Chunk、邻居扩展和 LLM contextual chunk generation。
- 将用户私有知识库上传为评测数据。
