// 会话级知识库挂载选择器：底部栏图标 + 勾选弹层。选中的库 id 写回会话，
// knowledge_search 缺省检索这些库（一个都不选时检索全部库）。
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Library, Check } from 'lucide-react'
import { kbListLibraries, onKbIndex, type KnowledgeLibrary } from './knowledgeBase'
import { IconButton } from '../components/Button'

export function KnowledgeBaseChip({
  value,
  onChange,
  disabled,
  layout = 'footer',
  anchorRef,
}: {
  value: string[]
  onChange: (ids: string[]) => void
  disabled?: boolean
  layout?: 'footer' | 'inline'
  // 弹层 portal 挂载到输入框容器，与项目弹窗共用同一锚点/方向/样式。
  anchorRef?: RefObject<HTMLDivElement | null>
}) {
  const [open, setOpen] = useState(false)
  const [libraries, setLibraries] = useState<KnowledgeLibrary[]>([])
  const [hasAny, setHasAny] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const popoverRef = useRef<HTMLDivElement>(null)

  const loadLibs = useCallback(async () => {
    try {
      const libs = await kbListLibraries()
      setLibraries(libs)
      setHasAny(libs.length > 0)
      // 清理已删除库留下的陈旧挂载 id（否则计数偏大且无法在弹层取消勾选）。
      const valid = value.filter((id) => libs.some((l) => l.id === id))
      if (valid.length !== value.length) onChange(valid)
    } catch {
      /* ignore */
    }
  }, [value, onChange])

  // 用 ref 让 onKbIndex 订阅保持稳定（只订阅一次），同时总能调到最新 loadLibs。
  const loadLibsRef = useRef(loadLibs)
  loadLibsRef.current = loadLibs

  // 初次评估 + 库变化(索引事件)时重评：保证创建/导入首个库后 chip 自动出现，无需重开聊天窗。
  useEffect(() => {
    void loadLibsRef.current()
    let cancelled = false
    let unlisten: (() => void) | undefined
    void onKbIndex(() => {
      void loadLibsRef.current()
    }).then((fn) => {
      if (cancelled) fn()
      else unlisten = fn
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  useEffect(() => {
    if (open) void loadLibsRef.current()
  }, [open])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node
      // 弹层经 portal 渲染到容器外，需同时排除按钮与弹层本身，否则点弹层会被判为外部点击而关闭。
      if (ref.current?.contains(t) || popoverRef.current?.contains(t)) return
      setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [open])

  const hasMounted = value.length > 0
  if (!hasAny && !hasMounted) return null

  const toggle = (id: string) => {
    onChange(value.includes(id) ? value.filter((x) => x !== id) : [...value, id])
  }

  // 欢迎页(inline)向下展开、对话页(footer)向上展开——与项目弹窗规则一致。
  const placement = layout === 'inline' ? 'top-full mt-1.5' : 'bottom-full mb-1.5'
  const origin = layout === 'inline' ? 'top left' : 'bottom left'

  const panel =
    open && anchorRef?.current
      ? createPortal(
          <div
            ref={popoverRef}
            className={`chat-motion-popover chat-popover-scroll absolute inset-x-0 z-40 max-h-[40vh] overflow-y-auto rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] p-1 shadow-[0_10px_24px_rgba(0,0,0,0.12)] dark:border-neutral-700 dark:bg-neutral-900 ${placement}`}
            style={{ ['--chat-popover-origin' as string]: origin }}
            data-tauri-drag-region="false"
            role="menu"
          >
            {libraries.length === 0 ? (
              <p className="px-2 py-2 text-[11px] text-neutral-500">在设置 · 知识库里先创建知识库。</p>
            ) : (
              <>
                <p className="px-2 py-1 text-[10.5px] text-neutral-400">
                  {hasMounted ? '勾选的库参与检索' : '未勾选则不检索任何库'}
                </p>
                {libraries.map((lib) => {
                  const checked = value.includes(lib.id)
                  return (
                    <button
                      key={lib.id}
                      type="button"
                      onClick={() => toggle(lib.id)}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] text-neutral-700 hover:bg-neutral-100 dark:text-neutral-200 dark:hover:bg-neutral-800"
                    >
                      <span
                        className={`grid size-4 shrink-0 place-items-center rounded border ${
                          checked
                            ? 'border-indigo-500 bg-indigo-500 text-white'
                            : 'border-neutral-300 dark:border-neutral-600'
                        }`}
                      >
                        {checked && <Check size={11} strokeWidth={3} />}
                      </span>
                      <span className="min-w-0 flex-1 truncate">{lib.name}</span>
                      <span className="shrink-0 text-[10.5px] text-neutral-400">{lib.docCount}</span>
                    </button>
                  )
                })}
              </>
            )}
          </div>,
          anchorRef.current,
        )
      : null

  return (
    <div className="relative" ref={ref}>
      <IconButton
        size="sm"
        shape="circle"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className={`relative focus-visible:ring-2 focus-visible:ring-neutral-300/60 dark:focus-visible:ring-neutral-600 ${
          open
            ? 'bg-neutral-200 text-neutral-700 dark:bg-neutral-700 dark:text-neutral-100'
            : hasMounted
              ? 'text-indigo-500 hover:bg-neutral-100 dark:text-indigo-300 dark:hover:bg-neutral-800'
              : 'text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800'
        }`}
        aria-expanded={open}
        aria-haspopup="menu"
        label={hasMounted ? `知识库 · 已挂载 ${value.length} 个` : '选择本会话使用的知识库'}
      >
        <Library size={15} strokeWidth={1.75} />
        {hasMounted && (
          <span className="absolute -right-0.5 -top-0.5 grid min-w-[14px] place-items-center rounded-full bg-indigo-500 px-1 text-[9px] font-bold leading-[14px] text-white">
            {value.length}
          </span>
        )}
      </IconButton>
      {panel}
    </div>
  )
}
