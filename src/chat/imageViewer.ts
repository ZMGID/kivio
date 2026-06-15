export type ChatImageViewerItem = {
  src: string
  alt?: string
  name?: string
  // 当 src 是缩略图时,提供 path + conversationId 让查看器按需懒加载全分辨率原图。
  path?: string | null
  conversationId?: string | null
}

const CHAT_IMAGE_VIEWER_EVENT = 'kivio-chat-open-image-viewer'

export function openChatImageViewer(item: ChatImageViewerItem) {
  window.dispatchEvent(new CustomEvent<ChatImageViewerItem>(CHAT_IMAGE_VIEWER_EVENT, {
    detail: item,
  }))
}

export function onChatImageViewerOpen(listener: (item: ChatImageViewerItem) => void): () => void {
  const handler = (event: Event) => {
    const detail = (event as CustomEvent<ChatImageViewerItem>).detail
    if (!detail?.src) return
    listener(detail)
  }
  window.addEventListener(CHAT_IMAGE_VIEWER_EVENT, handler)
  return () => window.removeEventListener(CHAT_IMAGE_VIEWER_EVENT, handler)
}
