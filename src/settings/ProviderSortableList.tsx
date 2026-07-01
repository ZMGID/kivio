import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { GripHorizontal } from 'lucide-react'
import type { ModelProvider } from '../api/tauri'
import { isProviderEnabled } from './utils'

type ProviderSortableListProps = {
  providers: ModelProvider[]
  selectedId: string | undefined
  lang: 'zh' | 'zh-TW' | 'en'
  providerNameLabel: string
  onSelect: (id: string) => void
  onReorder: (fromId: string, toId: string) => void
  /** 追加在真实供应商项之后、同一列表容器内的节点（如快速预设项），保证连续排列不被撑到底部。 */
  trailing?: ReactNode
}

export function ProviderSortableList({
  providers,
  selectedId,
  lang,
  providerNameLabel,
  onSelect,
  onReorder,
  trailing,
}: ProviderSortableListProps) {
  const [draggingId, setDraggingId] = useState<string | null>(null)
  const [overIndex, setOverIndex] = useState<number | null>(null)
  const [dragOffsetY, setDragOffsetY] = useState(0)
  const itemRefs = useRef(new Map<string, HTMLDivElement>())
  const dragStartY = useRef(0)
  const draggingIdRef = useRef<string | null>(null)
  const itemHeight = useRef(30)

  const draggingIndex = draggingId ? providers.findIndex((p) => p.id === draggingId) : -1

  const getIndexAtY = useCallback((clientY: number) => {
    for (let i = 0; i < providers.length; i++) {
      const el = itemRefs.current.get(providers[i].id)
      if (!el) continue
      const rect = el.getBoundingClientRect()
      if (clientY < rect.top + rect.height / 2) return i
    }
    return Math.max(0, providers.length - 1)
  }, [providers])

  useEffect(() => {
    if (!draggingId) return
    const prev = document.body.style.userSelect
    document.body.style.userSelect = 'none'
    return () => {
      document.body.style.userSelect = prev
    }
  }, [draggingId])

  const finishDrag = useCallback((clientY: number) => {
    const fromId = draggingIdRef.current
    if (!fromId) return
    const toIndex = getIndexAtY(clientY)
    const toId = providers[toIndex]?.id
    if (toId && fromId !== toId) onReorder(fromId, toId)
    draggingIdRef.current = null
    setDraggingId(null)
    setOverIndex(null)
    setDragOffsetY(0)
  }, [getIndexAtY, onReorder, providers])

  const handlePointerDown = (e: React.PointerEvent<HTMLButtonElement>, providerId: string, index: number) => {
    e.preventDefault()
    e.stopPropagation()
    const el = itemRefs.current.get(providerId)
    if (!el) return

    itemHeight.current = el.getBoundingClientRect().height || 30
    dragStartY.current = e.clientY
    draggingIdRef.current = providerId
    setDraggingId(providerId)
    setOverIndex(index)
    setDragOffsetY(0)

    const onMove = (ev: PointerEvent) => {
      setDragOffsetY(ev.clientY - dragStartY.current)
      setOverIndex(getIndexAtY(ev.clientY))
    }

    const onUp = (ev: PointerEvent) => {
      document.removeEventListener('pointermove', onMove)
      document.removeEventListener('pointerup', onUp)
      document.removeEventListener('pointercancel', onUp)
      finishDrag(ev.clientY)
    }

    document.addEventListener('pointermove', onMove)
    document.addEventListener('pointerup', onUp)
    document.addEventListener('pointercancel', onUp)
  }

  const getItemTransform = (index: number) => {
    if (draggingIndex < 0 || overIndex === null) return undefined
    const h = itemHeight.current
    if (index === draggingIndex) return `translateY(${dragOffsetY}px)`
    if (draggingIndex < overIndex) {
      if (index > draggingIndex && index <= overIndex) return `translateY(${-h}px)`
    } else if (draggingIndex > overIndex) {
      if (index >= overIndex && index < draggingIndex) return `translateY(${h}px)`
    }
    return undefined
  }

  const dragLabel = lang.startsWith('zh') ? '拖曳調整順序' : 'Drag to reorder'

  return (
    <div className={`kv-provider-list-items custom-scrollbar${draggingId ? ' is-sorting' : ''}`}>
      {providers.map((provider, index) => {
        const configured = provider.apiKeys.some((key) => key.trim())
        const isDragging = draggingId === provider.id
        const transform = getItemTransform(index)

        return (
          <div
            key={provider.id}
            ref={(el) => {
              if (el) itemRefs.current.set(provider.id, el)
              else itemRefs.current.delete(provider.id)
            }}
            className={`kv-provider-item ${selectedId === provider.id ? 'active' : ''}${isDragging ? ' is-dragging' : ''}`}
            style={transform ? { transform } : undefined}
            data-tauri-drag-region="false"
            role="button"
            tabIndex={0}
            onClick={() => onSelect(provider.id)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onSelect(provider.id)
              }
            }}
          >
            <span className="kv-provider-item-select">
              <span className={`kv-provider-dot ${!isProviderEnabled(provider) ? 'off' : configured ? 'on' : 'warn'}`} />
              <span className="kv-provider-name">{provider.name || providerNameLabel}</span>
            </span>
            <button
              type="button"
              className="kv-provider-drag-handle"
              aria-label={dragLabel}
              title={dragLabel}
              onPointerDown={(e) => handlePointerDown(e, provider.id, index)}
              onClick={(e) => e.stopPropagation()}
              data-tauri-drag-region="false"
            >
              <GripHorizontal size={13} strokeWidth={2} />
            </button>
          </div>
        )
      })}
      {trailing}
    </div>
  )
}
