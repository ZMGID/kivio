// 知识库（RAG）前端 API：库 CRUD、文档导入/列举/删除、索引进度事件。
// 后端命令在 src-tauri/src/chat/knowledge_base/。
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

const isTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

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
  if (!isTauri()) return []
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
  if (!isTauri()) return []
  return invoke<KnowledgeDocument[]>('kb_list_documents', { kbId })
}

export async function kbDeleteDocument(kbId: string, docId: string): Promise<void> {
  await invoke('kb_delete_document', { kbId, docId })
}

export async function kbUploadDocument(
  kbId: string,
  filePath: string
): Promise<KnowledgeDocument> {
  return invoke<KnowledgeDocument>('kb_upload_document', { kbId, filePath })
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

export async function onKbIndex(handler: (ev: KbIndexEvent) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {}
  return listen<KbIndexEvent>('kb-index', (event) => handler(event.payload))
}
