# RAG Retrieval Research Summary

## Decision

This task will optimize Kivio's measurable core retrieval pipeline before adding LLM query rewriting, parent-child chunks, knowledge graphs, or a new vector database.

The decision follows two principles:

1. Fix confirmed defects and establish retrieval evaluation before adding higher-variance techniques.
2. Keep each later technique independently measurable against the same baseline.

## Current Kivio findings

### Keyword recall is unintentionally phrase-only

`src-tauri/src/chat/knowledge_base/store.rs` wraps the complete query in double quotes before FTS5 `MATCH`. With the trigram tokenizer this behaves like a contiguous phrase query. A natural-language question therefore misses a relevant passage when the wording is not an exact substring.

Local SQLite reproduction:

```text
document: 退款需要在购买后七天内申请，并提供订单编号。
query:    "退款需要什么条件"       -> 0 hits
query:    退款 OR 订单编号          -> 1 hit
query:    "退款需要在购买后"       -> 1 hit
```

### Embedding requests do not distinguish query and document roles

`src-tauri/src/chat/knowledge_base/embeddings.rs` sends the same `{ model, input }` body for document chunks and queries. This omits provider/model-specific retrieval roles such as Voyage `input_type`, Jina retrieval tasks, E5 query/passage prefixes, or BGE query instructions.

### Relevance filtering is ineffective

`knowledge_search` retains results whose RRF score is greater than zero. Any candidate participating in a retrieval lane receives a positive RRF score, so the condition does not distinguish relevant from irrelevant results.

### Retrieval stages are coupled and opaque

- One raw query is reused for dense, lexical, and rerank stages.
- Default final TopK is 5.
- Rerank candidate count is `max(topK * 4, 20)`.
- Candidate recall, rerank output, and context output are not independently configured.
- Rerank failure silently degrades except for stderr logging.
- There is no retrieval test surface or versioned Recall@K evaluation set.

## External evidence

### RAGFlow

- Treats retrieval testing as a required tuning step.
- Exposes vector/keyword similarity, threshold, weights, Top N, rerank, cross-language retrieval, PageIndex, and optional knowledge graph retrieval.
- Supports parent-child chunks, using small child chunks for recall and larger parent chunks for answer context.

Sources:

- https://ragflow.io/docs/run_retrieval_test
- https://ragflow.io/docs/configure_child_chunking_strategy

### Dify

- Supports vector, full-text, and hybrid retrieval.
- Exposes TopK, score threshold, semantic/keyword weights, rerank, and retrieval testing.

Source: https://docs.dify.ai/en/use-dify/knowledge/create-knowledge/setting-indexing-methods

### Anthropic Contextual Retrieval

- Prepends chunk-specific context before embedding and BM25 indexing.
- Reports lower top-20 retrieval failure rates from contextual embeddings, contextual BM25, and reranking.
- Demonstrates broad first-stage recall followed by reranking rather than reranking only a very small candidate set.

Source: https://www.anthropic.com/engineering/contextual-retrieval

### Pinecone, Weaviate, Qdrant, Cohere

- Pinecone documents query/passage embedding asymmetry, metadata filtering, hybrid search, and two-stage reranking.
- Weaviate offers both ranked fusion and relative-score fusion, noting that rank-only fusion loses score-distance information.
- Qdrant supports dense+sparse hybrid, multi-stage queries, and late-interaction reranking.
- Cohere recommends evaluating rerank scores on representative queries before choosing a threshold.

Sources:

- https://docs.pinecone.io/guides/optimize/increase-relevance
- https://docs.weaviate.io/weaviate/concepts/search/hybrid-search
- https://qdrant.tech/documentation/search/hybrid-queries/
- https://docs.cohere.com/docs/reranking-best-practices

### LlamaIndex and Microsoft

- Query transformations include HyDE, rewriting, augmentation, decomposition, and multi-step retrieval.
- These are useful follow-ups but add LLM-dependent behavior and should be measured after the deterministic core is fixed.

Sources:

- https://developers.llamaindex.ai/python/framework/optimizing/advanced_retrieval/query_transformations/
- https://learn.microsoft.com/en-us/azure/architecture/ai-ml/guide/rag/rag-information-retrieval

## Prioritized conclusion

### In this task

1. Shared production/test retrieval service and evaluation harness.
2. Safe lexical query construction and searchable document metadata.
3. Provider-aware query/document embedding profiles.
4. Independent candidate, rerank, and context limits.
5. Score-kind-aware thresholds and explicit fallback diagnostics.
6. Deterministic duplicate/overlap reduction.
7. Retrieval Test UI and baseline metrics.

### Follow-up tasks after metrics exist

1. Standalone-query rewrite and keyword/entity extraction.
2. Multi-query, HyDE, and query decomposition.
3. Parent-child chunks, neighboring context, and contextual embeddings.
4. Sparse embeddings, ColBERT, PageIndex, or knowledge graph retrieval.
