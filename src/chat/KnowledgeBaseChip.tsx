// 会话级知识库挂载选择器：底部栏药丸 + 勾选弹层。选中的库 id 写回会话，
// knowledge_search 缺省检索这些库（一个都不选时检索全部库）。
import { useCallback, useEffect, useRef, useState } from 'react'
import { Library, Check, ChevronDown } from 'lucide-react'
import { kbListLibraries, onKbIndex, type KnowledgeLibrary } from './knowledgeBase'

export function KnowledgeBaseChip({
  value,
  onChange,
  disabled,
}: {
  value: string[]
  onChange: (ids: string[]) => void
  disabled?: boolean
}) {
  const [open, setOpen] = useState(false)
  const [libraries, setLibraries] = useState<KnowledgeLibrary[]>([])
  const [hasAny, setHasAny] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

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
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [open])

  const hasMounted = value.length > 0
  if (!hasAny && !hasMounted) return null

  const toggle = (id: string) => {
    onChange(value.includes(id) ? value.filter((x) => x !== id) : [...value, id])
  }

  const label = hasMounted ? `知识库 · ${value.length}` : '知识库'

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className={`inline-flex h-[26px] max-w-full items-center gap-1 rounded-full px-2 text-left text-[12px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-300/60 disabled:cursor-default disabled:opacity-50 dark:focus-visible:ring-neutral-600 ${
          open
            ? 'bg-neutral-200 text-neutral-800 dark:bg-neutral-700 dark:text-neutral-100'
            : hasMounted
              ? 'text-neutral-700 hover:bg-neutral-200/60 dark:text-neutral-200 dark:hover:bg-neutral-700/55'
              : 'text-neutral-500 hover:bg-neutral-200/50 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-700/55 dark:hover:text-neutral-100'
        }`}
        aria-expanded={open}
        aria-haspopup="menu"
        title="选择本会话使用的知识库"
      >
        <Library size={13} strokeWidth={1.75} className="shrink-0 text-neutral-500 dark:text-neutral-300" />
        <span className="min-w-0 truncate">{label}</span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-40 mb-1.5 max-h-[40vh] w-60 overflow-y-auto rounded-lg border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] p-1 shadow-[0_6px_18px_-16px_rgba(0,0,0,0.2),0_1px_4px_rgba(0,0,0,0.05)] dark:border-neutral-700 dark:bg-neutral-900">
          {libraries.length === 0 ? (
            <p className="px-2 py-2 text-[11px] text-neutral-500">在设置 · 知识库里先创建知识库。</p>
          ) : (
            <>
              <p className="px-2 py-1 text-[10.5px] text-neutral-400">
                {hasMounted ? '勾选的库参与检索' : '未勾选时检索全部库'}
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
        </div>
      )}
    </div>
  )
}
