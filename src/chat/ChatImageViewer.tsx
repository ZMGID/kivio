import { useEffect, useState } from 'react'
import { ArrowLeft, ImageIcon, Minus, Plus, RotateCcw } from 'lucide-react'
import type { ChatImageViewerItem } from './imageViewer'
import { loadArtifactDataUrl } from './attachmentPreview'

type ChatImageViewerProps = {
  item: ChatImageViewerItem
  onClose: () => void
}

export function ChatImageViewer({ item, onClose }: ChatImageViewerProps) {
  const [zoom, setZoom] = useState(1)
  // 先显示缩略图(item.src),若有 path 则懒加载全分辨率原图并替换。
  const [fullSrc, setFullSrc] = useState<string | null>(null)
  const title = item.name || item.alt || '图片附件'

  useEffect(() => {
    setZoom(1)
  }, [item.src])

  useEffect(() => {
    setFullSrc(null)
    if (!item.path) return
    let cancelled = false
    void loadArtifactDataUrl(
      { path: item.path, dataUrl: item.src },
      item.conversationId,
    ).then((src) => {
      if (!cancelled && src) setFullSrc(src)
    })
    return () => {
      cancelled = true
    }
  }, [item.path, item.conversationId, item.src])

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [onClose])

  return (
    <section className="flex min-h-0 flex-1 flex-col bg-[#f6f6f4] dark:bg-[#181818]" aria-label="图片查看">
      <div className="flex h-[52px] shrink-0 items-center gap-2 border-b border-neutral-200/80 bg-white/90 px-4 backdrop-blur dark:border-neutral-800 dark:bg-[#202020]/92">
        <button
          type="button"
          onClick={onClose}
          className="grid h-8 w-8 place-items-center rounded-md text-neutral-500 transition-colors hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
          title="返回对话"
          aria-label="返回对话"
        >
          <ArrowLeft size={18} strokeWidth={1.9} />
        </button>
        <div className="grid h-8 w-8 shrink-0 place-items-center rounded-md bg-neutral-100 text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
          <ImageIcon size={16} strokeWidth={1.8} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-medium text-neutral-800 dark:text-neutral-100">
            {title}
          </div>
          <div className="truncate text-[11px] text-neutral-400 dark:text-neutral-500">
            Esc 返回对话
          </div>
        </div>
        <div className="flex items-center gap-1 rounded-full border border-neutral-200 bg-neutral-50 p-1 dark:border-neutral-700 dark:bg-neutral-900">
          <button
            type="button"
            onClick={() => setZoom((value) => Math.max(0.5, Number((value - 0.25).toFixed(2))))}
            className="grid h-7 w-7 place-items-center rounded-full text-neutral-500 hover:bg-white hover:text-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
            title="缩小"
            aria-label="缩小"
          >
            <Minus size={15} strokeWidth={1.9} />
          </button>
          <span className="w-12 text-center text-[12px] tabular-nums text-neutral-500 dark:text-neutral-400">
            {Math.round(zoom * 100)}%
          </span>
          <button
            type="button"
            onClick={() => setZoom((value) => Math.min(3, Number((value + 0.25).toFixed(2))))}
            className="grid h-7 w-7 place-items-center rounded-full text-neutral-500 hover:bg-white hover:text-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
            title="放大"
            aria-label="放大"
          >
            <Plus size={15} strokeWidth={1.9} />
          </button>
          <button
            type="button"
            onClick={() => setZoom(1)}
            className="grid h-7 w-7 place-items-center rounded-full text-neutral-500 hover:bg-white hover:text-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
            title="重置缩放"
            aria-label="重置缩放"
          >
            <RotateCcw size={14} strokeWidth={1.9} />
          </button>
        </div>
      </div>
      <div className="custom-scrollbar min-h-0 flex-1 overflow-auto px-6 py-7">
        <div className="flex min-h-full items-center justify-center">
          <img
            src={fullSrc ?? item.src}
            alt={item.alt ?? ''}
            className="block rounded-lg bg-white shadow-sm ring-1 ring-black/10 dark:bg-neutral-950 dark:ring-white/10"
            style={{
              width: zoom <= 1 ? 'auto' : `${zoom * 100}%`,
              maxWidth: zoom <= 1 ? '100%' : 'none',
              maxHeight: zoom <= 1 ? 'calc(100vh - 8rem)' : 'none',
              // 连续放大/缩小步进（%↔%）平滑过渡；auto↔% 边界为 width 模型固有限制，瞬跳。
              transition: 'width var(--kv-dur-fast) var(--kv-ease-out)',
            }}
          />
        </div>
      </div>
    </section>
  )
}
