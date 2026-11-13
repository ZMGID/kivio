// 拉取请求行：两行摘要（标题 / 作者与分支 / 行数色块），点击就地展开详情
// （操作、检查、审查、描述、文件）。详情首次展开时加载，按 PR 更新时间缓存。
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Check, Copy, ExternalLink, FileDiff, GitBranch, Loader2, MessageSquare, RefreshCw, TriangleAlert, X } from 'lucide-react'
import type { I18n, Lang } from '../../components/i18n'
import { Button } from '../../components/Button'
import { notifyError } from '../../components/appNoticeStore'
import { api } from '../../api/tauri'
import { copyToClipboard } from '../../utils/clipboard'
import { ChatMarkdown } from '../ChatMarkdown'
import { dockApi } from './api'
import { relativeTime } from './gitReviewModel'
import { formatCount } from './pullRequestModel'
import { Avatar, ChecksIcon, Collapse, DiffBlocks, Highlighted, StateIcon } from './PullRequestParts'
import type { PullRequestDetail, PullRequestItem } from '../../api/dockContracts'

const detailCache = new Map<string, PullRequestDetail>()
const CHECKS_PREVIEW = 5
const FILES_PREVIEW = 6

function openLink(url: string) {
  void api.openExternal(url).catch((err) => notifyError(String(err)))
}

function PullRequestDetailView({ item, workdir, onBranch, t, onCheckedOut }: {
  item: PullRequestItem
  workdir: string
  onBranch: boolean
  t: I18n
  onCheckedOut: () => void
}) {
  const cacheKey = `${workdir}#${item.number}@${item.updatedAt}`
  const [loaded, setLoaded] = useState<{ key: string; detail: PullRequestDetail } | null>(null)
  const [failedKey, setFailedKey] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [checkingOut, setCheckingOut] = useState(false)
  const [copied, setCopied] = useState(false)
  const [bodyExpanded, setBodyExpanded] = useState(false)
  const [bodyClipped, setBodyClipped] = useState(false)
  const [allChecks, setAllChecks] = useState(false)
  const bodyRef = useRef<HTMLDivElement>(null)
  const detail = loaded?.key === cacheKey ? loaded.detail : detailCache.get(cacheKey) ?? null
  const failed = failedKey === cacheKey

  useEffect(() => {
    if (detailCache.has(cacheKey)) return
    let cancelled = false
    dockApi.pullRequestDetail(workdir, item.number)
      .then((next) => {
        detailCache.set(cacheKey, next)
        if (!cancelled) setLoaded({ key: cacheKey, detail: next })
      })
      .catch((err) => {
        if (cancelled) return
        setFailedKey(cacheKey)
        notifyError(String(err))
      })
    return () => { cancelled = true }
  }, [cacheKey, workdir, item.number, attempt])

  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1600)
    return () => window.clearTimeout(timer)
  }, [copied])

  useLayoutEffect(() => {
    const el = bodyRef.current
    if (el) setBodyClipped(el.scrollHeight > el.clientHeight + 4)
  }, [detail?.body])

  const checkout = async () => {
    setCheckingOut(true)
    try {
      const result = await dockApi.pullRequestCheckout(workdir, item.number)
      if (result.ok) onCheckedOut()
      else notifyError(result.message)
    } catch (err) {
      notifyError(String(err))
    } finally {
      setCheckingOut(false)
    }
  }

  const checks = detail?.checks ?? []
  const count = (status: string) => checks.filter((check) => check.status === status).length
  const passed = count('success')
  const bar = [
    { n: passed, color: '#22a35a' },
    { n: count('failure'), color: '#e5484d' },
    { n: count('pending'), color: '#f59e0b' },
  ].filter((part) => part.n > 0)
  const shownChecks = allChecks ? checks : checks.slice(0, CHECKS_PREVIEW)
  const reviewChip = {
    approved: { label: t.prReviewApproved, tone: 'is-success' },
    changes_requested: { label: t.prReviewChanges, tone: 'is-danger' },
    review_required: { label: t.prReviewRequired, tone: 'is-warning' },
    '': null,
  }[item.reviewDecision]
  const canCheckout = item.state === 'open' || item.state === 'draft'

  return (
    <div className="chat-pr-detail">
      <div className="chat-pr-actions">
        <Button size="sm" onClick={() => openLink(item.url)}>
          <ExternalLink size={12} />
          {t.prOpenInBrowser}
        </Button>
        {canCheckout && (
          <Button size="sm" disabled={onBranch || checkingOut} onClick={() => void checkout()}>
            {checkingOut ? <Loader2 size={12} className="animate-spin" /> : onBranch ? <Check size={12} /> : <GitBranch size={12} />}
            {checkingOut ? t.prCheckingOut : onBranch ? t.prOnBranch : t.prCheckout}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => void copyToClipboard(item.url).then((ok) => ok && setCopied(true))}>
          {copied ? <Check size={12} className="text-emerald-600 dark:text-emerald-400" /> : <Copy size={12} />}
          {copied ? t.prCopied : t.prCopyLink}
        </Button>
      </div>

      <div className="chat-pr-chips">
        <span className="chat-pr-chip" title={`${item.baseRef} ← ${item.headRef}`}>
          <GitBranch size={10} />
          <span className="max-w-[180px] truncate font-mono">{item.baseRef} ← {item.headRef}</span>
        </span>
        <span className="chat-pr-chip tabular-nums" title={t.prFilesChanged.replace('{n}', String(item.changedFiles))}>
          <FileDiff size={10} aria-hidden="true" />
          <span aria-hidden="true">{item.changedFiles}</span>
          <span className="sr-only">{t.prFilesChanged.replace('{n}', String(item.changedFiles))}</span>
        </span>
        {item.comments > 0 && (
          <span className="chat-pr-chip tabular-nums" title={t.prComments.replace('{n}', String(item.comments))}>
            <MessageSquare size={10} aria-hidden="true" />
            <span aria-hidden="true">{item.comments}</span>
            <span className="sr-only">{t.prComments.replace('{n}', String(item.comments))}</span>
          </span>
        )}
        {reviewChip && <span className={`chat-pr-chip ${reviewChip.tone}`}>{reviewChip.label}</span>}
        {item.conflicting && <span className="chat-pr-chip is-danger"><TriangleAlert size={10} />{t.prConflicting}</span>}
      </div>

      {!detail && !failed && (
        <div className="flex flex-col gap-1.5" aria-busy="true">
          <div className="kv-skeleton h-1 w-full rounded-full" />
          <div className="kv-skeleton h-3 w-3/4 rounded" />
          <div className="kv-skeleton h-3 w-1/2 rounded" />
          <div className="kv-skeleton mt-1 h-12 w-full rounded-md" />
        </div>
      )}
      {!detail && failed && (
        <Button size="sm" variant="ghost" className="self-start" onClick={() => { setFailedKey(null); setAttempt((value) => value + 1) }}>
          <RefreshCw size={12} />
          {t.prRetry}
        </Button>
      )}

      {detail && (
        <>
          <section>
            <div className="chat-pr-section-title">
              {t.prChecksTitle}
              {checks.length > 0 && (
                <span className="font-normal tabular-nums">
                  {t.prChecksSummary.replace('{passed}', String(passed)).replace('{total}', String(checks.length))}
                </span>
              )}
            </div>
            {checks.length === 0 ? (
              <div className="text-[12px] text-[var(--text-faint)]">{t.prNoChecks}</div>
            ) : (
              <>
                <div className="chat-pr-checkbar" aria-hidden="true">
                  {bar.map((part, index) => (
                    <span
                      key={part.color}
                      style={{ width: `${(part.n / checks.length) * 100}%`, background: part.color, animationDelay: `${index * 90}ms` }}
                    />
                  ))}
                </div>
                {shownChecks.map((check, index) => (
                  <button
                    key={`${check.workflow}/${check.name}/${index}`}
                    type="button"
                    className="chat-pr-check chat-motion-row"
                    style={{ ['--chat-motion-delay' as string]: `${Math.min(index, 8) * 30}ms` }}
                    disabled={!check.url}
                    title={check.url || undefined}
                    onClick={() => openLink(check.url)}
                  >
                    <ChecksIcon checks={check.status} t={t} size={12} />
                    <span className="min-w-0 flex-1 truncate">{check.name}</span>
                    {check.workflow && <span className="max-w-[40%] shrink-0 truncate text-[11px] text-[var(--text-faint)]">{check.workflow}</span>}
                  </button>
                ))}
                {checks.length > CHECKS_PREVIEW && (
                  <button type="button" className="chat-pr-check text-[var(--text-muted)]" onClick={() => setAllChecks((value) => !value)}>
                    <span className="w-3" />
                    {allChecks ? t.prShowLess : `+${checks.length - CHECKS_PREVIEW}`}
                  </button>
                )}
              </>
            )}
          </section>

          {detail.reviews.length > 0 && (
            <section>
              <div className="chat-pr-section-title">{t.prReviewsTitle}</div>
              <div className="chat-pr-chips">
                {detail.reviews.map((review) => {
                  const approved = review.state === 'approved'
                  const changes = review.state === 'changes_requested'
                  return (
                    <span key={review.author} className={`chat-pr-chip ${approved ? 'is-success' : changes ? 'is-danger' : ''}`}>
                      {approved ? <Check size={10} /> : changes ? <X size={10} /> : <MessageSquare size={10} />}
                      {review.author}
                    </span>
                  )
                })}
              </div>
            </section>
          )}

          <section>
            <div className="chat-pr-section-title">{t.prDescription}</div>
            {detail.body.trim() ? (
              <>
                <div ref={bodyRef} className={`chat-pr-body${bodyExpanded ? ' is-expanded' : ''}${bodyClipped ? ' is-clipped' : ''}`}>
                  <ChatMarkdown content={detail.body} />
                </div>
                {(bodyClipped || bodyExpanded) && (
                  <Button size="sm" variant="ghost" className="mt-1" onClick={() => setBodyExpanded((value) => !value)}>
                    {bodyExpanded ? t.prShowLess : t.prShowMore}
                  </Button>
                )}
              </>
            ) : (
              <div className="text-[12px] text-[var(--text-faint)]">{t.prNoDescription}</div>
            )}
          </section>

          {detail.files.length > 0 && (
            <section>
              <div className="chat-pr-section-title">{t.prFilesTitle}</div>
              {detail.files.slice(0, FILES_PREVIEW).map((file) => (
                <div key={file.path} className="chat-pr-file" title={file.path}>
                  <span className="chat-pr-file-path"><bdi>{file.path}</bdi></span>
                  <span className="shrink-0 tabular-nums">
                    <span className="chat-pr-add">+{formatCount(file.additions)}</span>{' '}
                    <span className="chat-pr-del">−{formatCount(file.deletions)}</span>
                  </span>
                </div>
              ))}
              {detail.files.length > FILES_PREVIEW && (
                <div className="text-[11px] text-[var(--text-faint)]">
                  {t.prMoreFiles.replace('{n}', String(detail.files.length - FILES_PREVIEW))}
                </div>
              )}
            </section>
          )}
        </>
      )}
    </div>
  )
}

export function PullRequestRow({ item, open, index, terms, workdir, currentBranch, lang, t, onToggle, onCheckedOut }: {
  item: PullRequestItem
  open: boolean
  /** 入场级联序号（封顶，避免长列表末尾等太久）。 */
  index: number
  terms: string[]
  workdir: string
  currentBranch: string
  lang: Lang
  t: I18n
  onToggle: () => void
  onCheckedOut: () => void
}) {
  // 首次展开后保持挂载：收起时详情随高度过渡一起淡出，再展开不重新加载。
  const [mounted, setMounted] = useState(open)
  if (open && !mounted) setMounted(true)
  const onBranch = !item.crossRepository && item.headRef === currentBranch
  const visibleLabels = item.labels.slice(0, 3)

  return (
    <div
      className={`chat-pr-row chat-motion-row${open ? ' is-open' : ''}`}
      style={{ ['--chat-motion-delay' as string]: `${Math.min(index, 12) * 22}ms` }}
    >
      <button type="button" className="chat-pr-row-main" aria-expanded={open} onClick={onToggle}>
        <StateIcon state={item.state} conflicting={item.conflicting} t={t} />
        <span className="min-w-0 flex-1">
          <span className="chat-pr-title">
            <span className="chat-pr-number">#{item.number}</span>
            <Highlighted text={item.title} terms={terms} />
            {item.checks !== 'none' && (
              <span className="ml-1 inline-flex translate-y-[2px]"><ChecksIcon checks={item.checks} t={t} size={12} /></span>
            )}
          </span>
          <span className="chat-pr-meta">
            <Avatar login={item.author} src={item.authorAvatar} />
            <span className="shrink-0">{item.author}</span>
            <span className="text-[var(--text-faint)]">·</span>
            <span className="chat-pr-meta-ref" title={item.headRef}>
              {onBranch && <GitBranch size={10} className="mr-0.5 inline -translate-y-px text-[var(--accent)]" />}
              {item.headRef}
            </span>
          </span>
          {visibleLabels.length > 0 && (
            <span className="chat-pr-labels">
              {visibleLabels.map((label) => (
                <span key={label.name} className="chat-pr-label" style={{ ['--label-color' as string]: `#${label.color || '888888'}` }}>
                  <span>{label.name}</span>
                </span>
              ))}
              {item.labels.length > 3 && <span className="chat-pr-chip">+{item.labels.length - 3}</span>}
            </span>
          )}
        </span>
        <span className="chat-pr-side">
          <span className="whitespace-nowrap">
            <span className="chat-pr-add">+{formatCount(item.additions)}</span>{' '}
            <span className="chat-pr-del">−{formatCount(item.deletions)}</span>
          </span>
          <DiffBlocks additions={item.additions} deletions={item.deletions} />
          <span className="chat-pr-time">{relativeTime(item.updatedAt, lang)}</span>
        </span>
      </button>
      <Collapse open={open}>
        {mounted && (
          <PullRequestDetailView item={item} workdir={workdir} onBranch={onBranch} t={t} onCheckedOut={onCheckedOut} />
        )}
      </Collapse>
    </div>
  )
}
