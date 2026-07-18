# Implementation Plan — 优化知识库检索效果

> 当前为规划草案；用户评审并完成范围决策前不得执行 `task.py start`。

## Phase A — Baseline and evaluation harness

- [ ] 提取统一 retrieval service 契约，先保持现有行为，`knowledge_search` 改为调用该服务。
- [ ] 添加固定测试文档、查询和 relevance 标注，覆盖中文、英文、混合语言、编号和无答案问题。
- [ ] 添加评测 runner，输出 Recall@5/10/20、MRR 和负样本误召回率。
- [ ] 记录修改前 baseline，保存到任务 research/ 或评测快照。
- [ ] 添加 Retrieval Test 后端 command 和 diagnostics 类型。

Validation:

```bash
cargo test --manifest-path src-tauri/Cargo.toml knowledge_base
```

## Phase B — Lexical recall repair

- [ ] 将 FTS query builder 提取为纯函数。
- [ ] 实现安全关键词 OR/AND、exact term/phrase 通道和 term 数上限。
- [ ] 将 doc name、heading path 纳入 FTS schema 或 searchable text。
- [ ] 增加 FTS schema version/migration 或可重建逻辑。
- [ ] 添加中文自然语言改写、英文改写、错误码、标点和 FTS 操作符测试。
- [ ] 对比 baseline 指标，确认关键词通道不再普遍为空。

Rollback point: 保留 legacy quoted-phrase feature flag，直到迁移与评测通过。

## Phase C — Embedding retrieval profiles

- [ ] 定义 Rust/TypeScript `EmbeddingRetrievalProfile`。
- [ ] 为现有 OpenAI-compatible 行为提供 legacy default。
- [ ] 支持 query/document input type、prefix 和受限 extra body。
- [ ] 为已知 Voyage/Jina/E5/BGE 类模型提供预设或模型元数据映射。
- [ ] 明确 profile 变更何时要求重建索引。
- [ ] 添加请求体单元测试和至少一个 query/document 非对称端到端 mock test。

Rollback point: library 可切回 legacy profile；禁止破坏旧库向量读取。

## Phase D — Candidate pipeline, rerank and threshold

- [ ] 将 candidateK、rerankTopK、contextTopK 加入配置和 sanitize/default。
- [ ] 保存 vector distance/BM25 rank/fusion score，不在 store 层过早丢失诊断。
- [ ] 对 rerank response 同时解析 order 与 relevance score。
- [ ] 实现 score-kind-aware threshold。
- [ ] rerank 失败返回显式 fallback 状态，同时保持聊天可降级运行。
- [ ] 添加多库总候选硬上限与耗时/成本保护。
- [ ] 通过评测确定默认参数，不凭空采用草案值。

## Phase E — Deduplication and context packing

- [ ] 实现同文档相邻/高重叠 Chunk 去重或合并。
- [ ] 记录 kept/duplicate/below-threshold/truncated decision。
- [ ] 实现按 contextTopK 和 Token 预算的最终上下文选择。
- [ ] 添加重复窗口、唯一答案窗口和多文档多样性测试。

## Phase F — Retrieval Test UI

- [ ] 在知识库/RAG 设置中增加 Retrieval Test 入口。
- [ ] 支持选择一个或多个库、输入查询和运行。
- [ ] 展示阶段排名、分数、耗时、threshold/duplicate decision 和 rerank fallback。
- [ ] 允许从测试结果跳转/识别文档，但本阶段不要求完整文档预览器。
- [ ] 添加前端类型检查和关键组件测试。

Validation:

```bash
npm run lint
npm run build
```

## Phase G — Full quality gate

- [ ] 运行知识库 Rust 单元/集成测试。
- [ ] 运行前端 lint/build/相关组件测试。
- [ ] 运行检索评测并与 baseline 对比。
- [ ] 验证旧库迁移、重建、删除和多库检索。
- [ ] 验证 rerank provider 失败、Embedding provider 缺失和无答案查询。
- [ ] 检查隐私日志，不出现 API Key 或完整私有文档。
- [ ] 按 Trellis 流程执行 `trellis-check`，再决定 spec 更新和提交。

## Files with elevated risk

- `src-tauri/src/chat/knowledge_base/store.rs` — SQLite schema、FTS 和融合核心。
- `src-tauri/src/chat/knowledge_base/embeddings.rs` — provider 请求兼容性和计费调用。
- `src-tauri/src/mcp/native_registry.rs` — 所有聊天原生工具共享注册表，避免扩大改动面。
- `src-tauri/src/settings.rs` / `src/api/tauri.ts` — 跨层设置契约与迁移。
- `src/settings/KnowledgeRagPanel.tsx` — 设置持久化与 UI 默认值。

## Planning gate still open

- [x] 根据调研结论确定本任务范围为核心检索修复；Query Rewrite 与父子 Chunk 延后为独立任务。
- [x] PRD 已完成 convergence pass，已移除已解决的范围问题和重复临时结论。
- [ ] 用户评审 `prd.md`、`design.md`、`implement.md`。
- [ ] 用户明确同意实施后，才运行 `task.py start`。
