import { invoke } from '@tauri-apps/api/core'
import type { Attachment, PendingAttachment } from './types'
import { isTauriRuntime } from './utils'

type AttachmentLike = Pick<Attachment, 'path' | 'name' | 'type'>

function isLocalAttachmentPath(path: string): boolean {
  return path.includes('/') || path.includes('\\')
}

// 已发送附件使用对话目录中的唯一文件名，内容不会再被覆盖。只缓存这类稳定身份；
// 草稿的绝对路径可能被外部程序改写，只在并发读取期间去重。
const CACHE_MAX_BYTES = 24 * 1024 * 1024
const CACHE_TTL_MS = 30_000
const completed = new Map<string, { data: string; bytes: number; expires: number }>()
const reading = new Map<string, Promise<string | null>>()
let completedBytes = 0

function cached(key: string): string | null {
  const item = completed.get(key)
  if (!item) return null
  completed.delete(key)
  completedBytes -= item.bytes
  if (item.expires <= Date.now()) return null
  completed.set(key, item)
  completedBytes += item.bytes
  return item.data
}

function remember(key: string, data: string): void {
  // JS 字符串以最多 2 字节/字符计，宁可少缓存，不低估内存预算。
  const bytes = data.length * 2
  if (bytes > CACHE_MAX_BYTES) return
  const previous = completed.get(key)
  if (previous) completedBytes -= previous.bytes
  completed.delete(key)
  completed.set(key, { data, bytes, expires: Date.now() + CACHE_TTL_MS })
  completedBytes += bytes
  for (const [entryKey, item] of completed) {
    if (completedBytes <= CACHE_MAX_BYTES && item.expires > Date.now()) break
    completed.delete(entryKey)
    completedBytes -= item.bytes
  }
}

async function readImageDataUrl(path: string, conversationId?: string | null): Promise<string | null> {
  const stable = !isLocalAttachmentPath(path) && !!conversationId
  const key = `${conversationId ?? ''}\u0000${path}`
  if (stable) {
    const hit = cached(key)
    if (hit) return hit
  }
  const pending = reading.get(key)
  if (pending) return pending
  const request = (async () => {
    try {
      const result = await invoke<{ success: boolean; data?: string; error?: string }>(
        'chat_read_attachment',
        { conversationId: isLocalAttachmentPath(path) ? null : conversationId ?? null, path },
      )
      if (!result.success || !result.data) return null
      if (stable) remember(key, result.data)
      return result.data
    } catch (err) {
      console.warn('Failed to load attachment preview:', err)
      return null
    } finally {
      reading.delete(key)
    }
  })()
  reading.set(key, request)
  return request
}

export async function loadAttachmentDataUrl(
  attachment: AttachmentLike,
  conversationId?: string | null,
): Promise<string | null> {
  if (!isTauriRuntime() || attachment.type !== 'image') return null
  const previewConversationId = isLocalAttachmentPath(attachment.path) ? null : conversationId
  return readImageDataUrl(attachment.path, previewConversationId)
}

export async function openAttachment(
  attachment: AttachmentLike,
  conversationId?: string | null,
): Promise<void> {
  if (!isTauriRuntime()) return
  await invoke('chat_open_attachment', {
    conversationId: conversationId ?? null,
    path: attachment.path,
  })
}

type ArtifactLike = {
  path?: string | null
  data_url?: string
  dataUrl?: string
}

function artifactInlineDataUrl(artifact: ArtifactLike): string {
  return artifact.dataUrl ?? artifact.data_url ?? ''
}

// 图片 artifact 现在以"磁盘整图 + path + 内联缩略图"存储。需要全分辨率(点开查看器)时,
// 用 path 经 chat_read_attachment 懒加载;无 path 的老 artifact 直接回退到内联 data_url。
// path 有两种形态：外置附件是裸文件名（须带 conversationId 解析到附件目录），
// present_artifacts 等落盘成果是含分隔符的绝对路径（须**不带** conversationId——
// 带了会被后端"附件目录裸文件名"的校验拒掉，走绝对路径分支才读得到）。
export async function loadArtifactDataUrl(
  artifact: ArtifactLike,
  conversationId?: string | null,
): Promise<string | null> {
  const inline = artifactInlineDataUrl(artifact)
  return (await loadArtifactOriginalDataUrl(artifact, conversationId)) ?? (inline || null)
}

/** 复制和另存必须读到原图；失败时不能把缩略图当成成功结果。 */
export async function loadArtifactOriginalDataUrl(
  artifact: ArtifactLike,
  conversationId?: string | null,
): Promise<string | null> {
  const path = artifact.path ?? ''
  const isBareName = !!path && !path.includes('/') && !path.includes('\\')
  if (!isTauriRuntime() || !path || (isBareName && !conversationId)) return null
  return readImageDataUrl(path, isBareName ? conversationId : null)
}

export type DisplayAttachment = Attachment | PendingAttachment
