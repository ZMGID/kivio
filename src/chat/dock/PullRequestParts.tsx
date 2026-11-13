// 拉取请求面板的展示小件：状态图标、检查图标、头像、高亮、行数色块、状态分段、筛选菜单、空态。
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import {
  Check,
  CircleCheck,
  CircleDashed,
  CircleX,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
  TriangleAlert,
  X,
} from 'lucide-react'
import type { I18n } from '../../components/i18n'
import { motionClass } from './workspacePanelMotion'
import { DEFAULT_FILTERS, activeFilterCount, diffBlocks, highlightRanges, type PullRequestFilters } from './pullRequestModel'
import type { PullRequestChecks, PullRequestState, PullRequestStateFilter } from '../../api/dockContracts'

const STATE_ICON: Record<PullRequestState, { icon: typeof GitPullRequest; className: string }> = {
  open: { icon: GitPullRequest, className: 'text-success' },
  draft: { icon: GitPullRequestDraft, className: 'text-neutral-400 dark:text-neutral-500' },
  merged: { icon: GitMerge, className: 'text-violet-600 dark:text-violet-400' },
  closed: { icon: GitPullRequestClosed, className: 'text-danger' },
}

export function StateIcon({ state, conflicting, t }: { state: PullRequestState; conflicting: boolean; t: I18n }) {
  const { icon: Icon, className } = STATE_ICON[state]
  const label = { open: t.prStateOpen, draft: t.prStateDraft, merged: t.prStateMerged, closed: t.prStateClosed }[state]
  return (
    <span className="chat-pr-state" title={conflicting ? `${label} · ${t.prConflicting}` : label}>
      <Icon size={15} strokeWidth={1.9} className={className} aria-hidden="true" />
      {conflicting && (
        <span className="chat-pr-state-conflict" aria-hidden="true">
          <TriangleAlert size={9} strokeWidth={2.4} />
        </span>
      )}
    </span>
  )
}

export function ChecksIcon({ checks, t, size = 13 }: { checks: PullRequestChecks | 'skipped'; t: I18n; size?: number }) {
  const common = { size, 'aria-hidden': false, role: 'img' } as const
  if (checks === 'success') {
    return <CircleCheck {...common} strokeWidth={2} className="shrink-0 text-success" aria-label={t.prChecksSuccess} />
  }
  if (checks === 'failure') {
    return <CircleX {...common} strokeWidth={2} className="shrink-0 text-danger" aria-label={t.prChecksFailure} />
  }
  if (checks === 'pending') {
    return <CircleDashed {...common} strokeWidth={2.2} className="chat-pr-pending shrink-0" aria-label={t.prChecksPending} />
  }
  if (checks === 'skipped') {
    return <CircleDashed {...common} strokeWidth={2} className="shrink-0 text-neutral-400" aria-label={t.prChecksSkipped} />
  }
  return null
}

function avatarHue(login: string) {
  let hash = 0
  for (const char of login) hash = (hash * 31 + char.charCodeAt(0)) | 0
  return Math.abs(hash) % 360
}

export function Avatar({ login, src, size = 14 }: { login: string; src: string; size?: number }) {
  const [failed, setFailed] = useState(false)
  const style: CSSProperties = { width: size, height: size }
  if (!src || failed) {
    return (
      <span className="chat-pr-avatar" style={{ ...style, background: `hsl(${avatarHue(login)} 45% 55%)` }} aria-hidden="true">
        {login.slice(0, 1).toUpperCase()}
      </span>
    )
  }
  return (
    <img
      src={src}
      alt=""
      className="chat-pr-avatar"
      style={style}
      loading="lazy"
      referrerPolicy="no-referrer"
      draggable={false}
      onError={() => setFailed(true)}
    />
  )
}

export function Highlighted({ text, terms }: { text: string; terms: string[] }) {
  const ranges = terms.length ? highlightRanges(text, terms) : []
  if (!ranges.length) return <>{text}</>
  const parts: ReactNode[] = []
  let cursor = 0
  for (const [start, end] of ranges) {
    if (start > cursor) parts.push(text.slice(cursor, start))
    parts.push(<mark key={start} className="chat-pr-mark">{text.slice(start, end)}</mark>)
    cursor = end
  }
  if (cursor < text.length) parts.push(text.slice(cursor))
  return <>{parts}</>
}

export function DiffBlocks({ additions, deletions }: { additions: number; deletions: number }) {
  return (
    <span className="chat-pr-blocks" aria-hidden="true">
      {diffBlocks(additions, deletions).map((block, index) => (
        <i key={index} className={block === 'neutral' ? '' : `is-${block}`} style={{ ['--i' as string]: index }} />
      ))}
    </span>
  )
}

const STATE_FILTERS: PullRequestStateFilter[] = ['open', 'merged', 'closed', 'all']

/** 状态分段：选中片段在轨道上滑动；左右方向键切换。 */
export function StateSegment({ value, count, onChange, t }: {
  value: PullRequestStateFilter
  count: number | null
  onChange: (value: PullRequestStateFilter) => void
  t: I18n
}) {
  const trackRef = useRef<HTMLDivElement>(null)
  const [thumb, setThumb] = useState<{ x: number; w: number; settled: boolean } | null>(null)
  const labels: Record<PullRequestStateFilter, string> = {
    open: t.prStateOpen,
    merged: t.prStateMerged,
    closed: t.prStateClosed,
    all: t.prStateAll,
  }

  const measure = useCallback(() => {
    const button = trackRef.current?.querySelector<HTMLElement>('[aria-checked="true"]')
    if (!button) return
    // 首次测量不播过渡，避免选中片段从左侧滑入。
    setThumb((prev) => ({ x: button.offsetLeft, w: button.offsetWidth, settled: prev !== null }))
  }, [])

  useLayoutEffect(measure, [measure, value, count, t])
  useEffect(() => {
    const track = trackRef.current
    if (!track || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(track)
    return () => observer.disconnect()
  }, [measure])

  return (
    <div ref={trackRef} role="radiogroup" aria-label={t.prStateLabel} className="chat-pr-segment">
      <span
        aria-hidden="true"
        className={`chat-pr-segment-thumb${thumb?.settled ? '' : ' is-measuring'}`}
        style={{ ['--seg-x' as string]: `${thumb?.x ?? 0}px`, ['--seg-w' as string]: `${thumb?.w ?? 0}px` }}
      />
      {STATE_FILTERS.map((item) => (
        <button
          key={item}
          type="button"
          role="radio"
          aria-checked={value === item}
          tabIndex={value === item ? 0 : -1}
          onClick={() => onChange(item)}
          onKeyDown={(event) => {
            if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') return
            event.preventDefault()
            const index = STATE_FILTERS.indexOf(value) + (event.key === 'ArrowRight' ? 1 : -1)
            const next = STATE_FILTERS[(index + STATE_FILTERS.length) % STATE_FILTERS.length]
            onChange(next)
            trackRef.current?.querySelectorAll<HTMLButtonElement>('button')[STATE_FILTERS.indexOf(next)]?.focus()
          }}
        >
          {labels[item]}
          {value === item && count !== null && <span key={`${item}-${count}`} className="chat-pr-segment-count">{count}</span>}
        </button>
      ))}
    </div>
  )
}

/** 筛选菜单：CI、审查、草稿三组单选；锚在按钮下方右对齐。 */
export function FiltersMenu({ anchor, filters, closing, onChange, onClose, t }: {
  anchor: HTMLElement
  filters: PullRequestFilters
  closing: boolean
  onChange: (filters: PullRequestFilters) => void
  onClose: () => void
  t: I18n
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos] = useState(() => {
    const rect = anchor.getBoundingClientRect()
    return { top: rect.bottom + 6, right: Math.max(8, window.innerWidth - rect.right) }
  })

  useEffect(() => {
    if (closing) return
    ref.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus({ preventScroll: true })
    const onDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (!ref.current?.contains(target) && !anchor.contains(target)) onClose()
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      onClose()
      anchor.focus()
    }
    document.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [anchor, closing, onClose])

  const sections: { key: keyof PullRequestFilters; title: string; options: { value: string; label: string; icon?: ReactNode }[] }[] = [
    {
      key: 'checks',
      title: t.prFilterChecks,
      options: [
        { value: 'any', label: t.prFilterAny },
        { value: 'success', label: t.prChecksSuccess, icon: <ChecksIcon checks="success" t={t} size={12} /> },
        { value: 'failure', label: t.prChecksFailure, icon: <ChecksIcon checks="failure" t={t} size={12} /> },
        { value: 'pending', label: t.prChecksPending, icon: <ChecksIcon checks="pending" t={t} size={12} /> },
      ],
    },
    {
      key: 'review',
      title: t.prFilterReview,
      options: [
        { value: 'any', label: t.prFilterAny },
        { value: 'approved', label: t.prReviewApproved },
        { value: 'changes_requested', label: t.prReviewChanges },
        { value: 'review_required', label: t.prReviewRequired },
      ],
    },
    {
      key: 'draft',
      title: t.prFilterDraft,
      options: [
        { value: 'any', label: t.prFilterAny },
        { value: 'only', label: t.prDraftOnly },
        { value: 'exclude', label: t.prDraftExclude },
      ],
    },
  ]

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={t.prFilters}
      data-tauri-drag-region="false"
      className={`kv-menu fixed z-[60] w-48 ${motionClass(closing)}`}
      style={{ top: pos.top, right: pos.right, ['--chat-popover-origin' as string]: 'top right' }}
    >
      {sections.map((section, index) => (
        <div key={section.key} role="group" aria-label={section.title}>
          {index > 0 && <div className="kv-menu-sep" />}
          <div className="kv-menu-label">{section.title}</div>
          {section.options.map((option) => {
            const checked = filters[section.key] === option.value
            return (
              <button
                key={option.value}
                type="button"
                role="menuitemradio"
                aria-checked={checked}
                className="kv-menu-item"
                onClick={() => onChange({ ...filters, [section.key]: option.value })}
              >
                <span className="flex size-3.5 shrink-0 items-center justify-center text-[var(--accent)]">
                  {checked && <Check size={13} strokeWidth={2.2} />}
                </span>
                <span className="min-w-0 flex-1 truncate">{option.label}</span>
                {/* 包一层：菜单项直属 svg 会被统一压成弱化色，状态图标需保留语义色。 */}
                {option.icon && <span className="flex shrink-0">{option.icon}</span>}
              </button>
            )
          })}
        </div>
      ))}
      <div className="kv-menu-sep" />
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        disabled={activeFilterCount(filters) === 0}
        onClick={() => onChange(DEFAULT_FILTERS)}
      >
        <X />
        <span>{t.prFiltersReset}</span>
      </button>
    </div>,
    document.body,
  )
}

export function EmptyState({ icon, title, hint, children }: { icon: ReactNode; title: string; hint?: string; children?: ReactNode }) {
  return (
    <div className="chat-pr-empty">
      <span className="chat-pr-orbit" aria-hidden="true">{icon}</span>
      <div className="chat-pr-empty-title">{title}</div>
      {hint && <div className="chat-pr-empty-hint">{hint}</div>}
      {children && <div className="mt-2 flex flex-wrap justify-center gap-2">{children}</div>}
    </div>
  )
}

/** 高度折叠：grid 行 0fr↔1fr 过渡；收起时设 inert，屏外内容不进入 Tab 顺序。 */
export function Collapse({ open, children }: { open: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    if (ref.current) ref.current.inert = !open
  }, [open])
  return (
    <div ref={ref} className={`chat-pr-collapse${open ? ' is-open' : ''}`}>
      <div>{children}</div>
    </div>
  )
}

export function SkeletonRows() {
  return (
    <div aria-busy="true" className="pt-2">
      {[0.82, 0.64, 0.74, 0.55, 0.7].map((width, index) => (
        <div key={index} className="chat-pr-skeleton-row" style={{ opacity: 1 - index * 0.14 }}>
          <div className="kv-skeleton size-4 shrink-0 rounded-full" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <div className="kv-skeleton h-3 rounded" style={{ width: `${width * 100}%` }} />
            <div className="kv-skeleton h-2.5 w-2/5 rounded" />
          </div>
          <div className="kv-skeleton h-3 w-10 shrink-0 rounded" />
        </div>
      ))}
    </div>
  )
}
