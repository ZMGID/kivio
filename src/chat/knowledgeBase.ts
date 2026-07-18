// 知识库（RAG）前端 API：库 CRUD、文档导入/列举/删除、索引进度事件。
// 后端命令在 src-tauri/src/chat/knowledge_base/。
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { isTauriRuntime } from '../api/tauri'

export interface KnowledgeLibrary {
  id: string
  name: string
  embeddingProviderId: string
  embeddingModel: string
  embeddingDim: number
  createdAt: number
  updatedAt: number
  docCount: number
  chunkCount: number
  /** 每次 embedding 请求打包的片段数；0/缺省 = 用内置默认（64）。 */
  embedBatchSize?: number
}

export type DocStatus = 'indexing' | 'ready' | 'error'

export interface KnowledgeDocument {
  id: string
  name: string
  sizeBytes: number
  hash: string
  chunkCount: number
  status: DocStatus
  error?: string | null
  createdAt: number
}

// `kb-index` 事件 payload（后端 ingest.rs::KbIndexEvent 的镜像）。
export interface KbIndexEvent {
  kbId: string
  docId: string
  status: DocStatus
  indexed: number
  total: number
  error?: string
}

export async function kbListLibraries(): Promise<KnowledgeLibrary[]> {
  if (!isTauriRuntime()) return []
  return invoke<KnowledgeLibrary[]>('kb_list_libraries')
}

export async function kbCreateLibrary(
  name: string,
  providerId: string,
  model: string
): Promise<KnowledgeLibrary> {
  return invoke<KnowledgeLibrary>('kb_create_library', { name, providerId, model })
}

export async function kbRenameLibrary(kbId: string, name: string): Promise<void> {
  await invoke('kb_rename_library', { kbId, name })
}

export async function kbDeleteLibrary(kbId: string): Promise<void> {
  await invoke('kb_delete_library', { kbId })
}

export async function kbListDocuments(kbId: string): Promise<KnowledgeDocument[]> {
  if (!isTauriRuntime()) return []
  return invoke<KnowledgeDocument[]>('kb_list_documents', { kbId })
}

export async function kbDeleteDocument(kbId: string, docId: string): Promise<void> {
  await invoke('kb_delete_document', { kbId, docId })
}

// ===== Retrieval Test (D1) — mirrors retrieval.rs diagnostics contract =====

export type RetrievalDecision = 'kept' | 'duplicate' | 'below_threshold' | 'truncated'

export interface RetrievalCandidate {
  kbId: string
  docId: string
  chunkId: string
  docName: string
  headingPath?: string
  text: string
  orderIndex: number
  vectorRank?: number
  vectorDistance?: number
  keywordRank?: number
  fusedScore: number
  rerankScore?: number
  finalRank?: number
  decision: RetrievalDecision
}

export interface RetrievalTimings {
  embedMs: number
  searchMs: number
  rerankMs: number
  totalMs: number
}

export type RerankStatus =
  | { state: 'off' }
  | { state: 'ok' }
  | { state: 'failed'; error: string }

export interface RetrievalEffectiveConfig {
  candidateK: number
  rerankTopK: number
  contextTopK: number
  weightVector: number
  weightKeyword: number
  rerankOn: boolean
  minScore: number
}

export interface RetrievalResponse {
  candidates: RetrievalCandidate[]
  timings: RetrievalTimings
  rerankStatus: RerankStatus
  effectiveConfig: RetrievalEffectiveConfig
}

/** Run a diagnostic retrieval through the same core as `knowledge_search`. */
export async function kbRetrievalTest(
  kbIds: string[],
  query: string,
  topK?: number
): Promise<RetrievalResponse> {
  return invoke<RetrievalResponse>('kb_retrieval_test', { kbIds, query, topK })
}

export async function kbUploadDocument(
  kbId: string,
  filePath: string
): Promise<KnowledgeDocument> {
  return invoke<KnowledgeDocument>('kb_upload_document', { kbId, filePath })
}

export async function kbImportUrl(kbId: string, url: string): Promise<KnowledgeDocument> {
  return invoke<KnowledgeDocument>('kb_import_url', { kbId, url })
}

export async function kbReindexLibrary(kbId: string): Promise<void> {
  await invoke('kb_reindex_library', { kbId })
}

export async function kbUpdateEmbedding(
  kbId: string,
  providerId: string,
  model: string
): Promise<void> {
  await invoke('kb_update_embedding', { kbId, providerId, model })
}

/** 设置某库每次 embedding 请求的片段数（0 = 用默认）。只影响后续索引，不重建。 */
export async function kbSetEmbedBatchSize(kbId: string, size: number): Promise<void> {
  await invoke('kb_set_embed_batch_size', { kbId, size })
}

export async function onKbIndex(handler: (ev: KbIndexEvent) => void): Promise<UnlistenFn> {
  if (!isTauriRuntime()) return () => {}
  return listen<KbIndexEvent>('kb-index', (event) => handler(event.payload))
}
