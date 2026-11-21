import { memo, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { createPortal } from 'react-dom'
import { Boxes, Eraser } from 'lucide-react'
import { useT } from '../components/i18n'
import { messageNavigatorProximityWidth, type MessageNavigatorNode } from './messageNavigator'

// The rail has fixed-height ticks, including compaction and clear markers.
const TICK_HEIGHT = 13

interface PreviewAnchor {
  node: MessageNavigatorNode
  top: number
  left: number
}

interface MessageNavigatorProps {
  nodes: MessageNavigatorNode[]
  activeNodeId: string | null
  visibleNodeIds: string[]
  onNavigate: (node: MessageNavigatorNode) => void
}

function nodesEqual(a: MessageNavigatorNode[], b: MessageNavigatorNode[]): boolean {
  if (a === b) return true
  if (a.length !== b.length) return false
  return a.every((node, index) => {
    const other = b[index]
    return node.kind === other.kind
      && node.id === other.id
      && node.targetRenderIndex === other.targetRenderIndex
      && node.title === other.title
      && node.answerPreview === other.answerPreview
      && node.modelLabel === other.modelLabel
  })
}

function MessageNavigatorBase({
  nodes,
  activeNodeId,
  visibleNodeIds,
  onNavigate,
}: MessageNavigatorProps) {
  const t = useT()
  const viewportRef = useRef<HTMLDivElement>(null)
  const browsingRailRef = useRef(false)
  const proximityFrameRef = useRef<number | null>(null)
  const proximityPointerYRef = useRef(0)
  const [preview, setPreview] = useState<PreviewAnchor | null>(null)
  const visibleNodeIdSet = new Set(visibleNodeIds)
  const windowed = nodes.length > 64
  const virtualizer = useVirtualizer({
    count: nodes.length,
    getScrollElement: () => viewportRef.current,
    estimateSize: () => TICK_HEIGHT,
    enabled: windowed,
    overscan: 6,
  })
  const ticks = virtualizer.getVirtualItems()
  const first = windowed ? ticks[0]?.index ?? 0 : 0
  const end = windowed ? (ticks.at(-1)?.index ?? -1) + 1 : nodes.length
  const activeIndex = activeNodeId ? nodes.findIndex(node => node.id === activeNodeId) : -1

  useEffect(() => () => {
    if (proximityFrameRef.current != null) cancelAnimationFrame(proximityFrameRef.current)
  }, [])

  // 滚轮只滚导航条本身的小横杠列表，绝不切换消息、也绝不带动主列表。
  // React onWheel 多为 passive，preventDefault 无效；必须非 passive 原生监听，
  // 自己改 scrollTop，吞掉事件。
  useEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      event.stopPropagation()
      let delta = event.deltaY
      if (event.deltaMode === 1) delta *= 13
      else if (event.deltaMode === 2) delta *= el.clientHeight
      if (delta === 0) return
      browsingRailRef.current = true
      el.scrollTop += delta
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [])

  useLayoutEffect(() => {
    const viewport = viewportRef.current
    if (!viewport || activeIndex < 0 || browsingRailRef.current) return
    // Scroll only the rail. scrollIntoView would also scroll its ancestors.
    const top = activeIndex * TICK_HEIGHT
    const bottom = top + TICK_HEIGHT
    if (top < viewport.scrollTop) viewport.scrollTop = top
    else if (bottom > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = bottom - viewport.clientHeight
    }
  }, [activeIndex, activeNodeId])

  const showPreview = (node: MessageNavigatorNode, button: HTMLButtonElement) => {
    const rect = button.getBoundingClientRect()
    const center = rect.top + rect.height / 2
    // 导航条贴右侧滚动条，预览气泡向左弹出，避免超出右边界。
    const previewWidth = Math.min(320, window.innerWidth - 32)
    setPreview({
      node,
      top: Math.min(window.innerHeight - 96, Math.max(96, center)),
      left: Math.max(16, rect.left - 12 - previewWidth),
    })
  }

  const applyPointerProximity = () => {
    proximityFrameRef.current = null
    const pointerY = proximityPointerYRef.current
    const buttons = viewportRef.current?.querySelectorAll<HTMLElement>('.chat-message-navigator-node')
    buttons?.forEach((button) => {
      const rect = button.getBoundingClientRect()
      const distance = Math.abs(pointerY - (rect.top + rect.height / 2))
      const width = messageNavigatorProximityWidth(distance)
      button.style.setProperty('--message-navigator-node-width', `${width.toFixed(2)}px`)
    })
  }

  const handlePointerMove = (event: React.MouseEvent) => {
    proximityPointerYRef.current = event.clientY
    if (proximityFrameRef.current != null) return
    proximityFrameRef.current = requestAnimationFrame(applyPointerProximity)
  }

  const clearPointerProximity = () => {
    browsingRailRef.current = false
    if (proximityFrameRef.current != null) cancelAnimationFrame(proximityFrameRef.current)
    proximityFrameRef.current = null
    viewportRef.current?.querySelectorAll<HTMLElement>('.chat-message-navigator-node')
      .forEach((button) => button.style.removeProperty('--message-navigator-node-width'))
    setPreview(null)
  }

  return (
    <>
      <aside className="chat-message-navigator" aria-label={t.chatTurnNavigator}>
        <div
          ref={viewportRef}
          className="chat-message-navigator-viewport"
          onMouseMove={handlePointerMove}
          onMouseLeave={clearPointerProximity}
        >
          <div
            className="chat-message-navigator-track"
            style={windowed ? { paddingTop: first * TICK_HEIGHT, paddingBottom: (nodes.length - end) * TICK_HEIGHT } : undefined}
          >
            {nodes.slice(first, end).map((node, offset) => {
              const active = node.id === activeNodeId
              return (
                <button
                  key={node.id}
                  type="button"
                  data-message-navigator-id={node.id}
                  className={`chat-message-navigator-node ${active ? 'is-active' : ''} ${visibleNodeIdSet.has(node.id) ? 'is-visible' : ''} ${node.kind === 'compaction' || node.kind === 'clear' ? 'is-compaction' : ''}`}
                  aria-current={active ? 'location' : undefined}
                  aria-label={
                    node.kind === 'compaction'
                      ? t.chatCompactionSummaryLabel
                      : node.kind === 'clear'
                        ? t.contextClearDividerAria
                        : t.chatTurnLabel
                            .replace('{n}', String(first + offset + 1))
                            .replace('{title}', node.title)
                  }
                  onClick={() => {
                    browsingRailRef.current = false
                    onNavigate(node)
                  }}
                  onMouseEnter={(event) => showPreview(node, event.currentTarget)}
                  onFocus={(event) => showPreview(node, event.currentTarget)}
                  onBlur={() => setPreview(null)}
                >
                  {node.kind === 'compaction' || node.kind === 'clear' ? (
                    <span className="chat-message-navigator-compaction-mark" aria-hidden="true">
                      <span />
                      <span />
                    </span>
                  ) : (
                    <span className="chat-message-navigator-tick" aria-hidden="true" />
                  )}
                </button>
              )
            })}
          </div>
        </div>
      </aside>
      {preview && createPortal(
        <div
          className="chat-message-navigator-preview"
          style={{ top: preview.top, left: preview.left }}
          role="tooltip"
        >
          <div className="chat-message-navigator-preview-title">
            {preview.node.kind === 'compaction' && <Boxes size={14} aria-hidden="true" />}
            {preview.node.kind === 'clear' && <Eraser size={14} aria-hidden="true" />}
            <span>{preview.node.title || t.chatUntitledMessage}</span>
          </div>
          {preview.node.answerPreview && (
            <p className="chat-message-navigator-preview-answer">{preview.node.answerPreview}</p>
          )}
          {preview.node.modelLabel && (
            <div className="chat-message-navigator-preview-model">{preview.node.modelLabel}</div>
          )}
        </div>,
        document.body,
      )}
    </>
  )
}

export const MessageNavigator = memo(
  MessageNavigatorBase,
  (prev, next) => (
    prev.activeNodeId === next.activeNodeId
    && prev.visibleNodeIds.length === next.visibleNodeIds.length
    && prev.visibleNodeIds.every((id, index) => id === next.visibleNodeIds[index])
    && prev.onNavigate === next.onNavigate
    && nodesEqual(prev.nodes, next.nodes)
  ),
)
