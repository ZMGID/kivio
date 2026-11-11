// 当前分支卡片：分支已有 PR 时显示该 PR（可展开）；没有 PR 且不在默认分支时，
// 就地展开创建表单。提交由后端先推送未推送的提交，再用 gh 创建 PR。
import { useEffect, useState } from 'react'
import { GitBranch, GitPullRequestCreate, Loader2 } from 'lucide-react'
import type { I18n, Lang } from '../../components/i18n'
import { Button } from '../../components/Button'
import { notifyError } from '../../components/appNoticeStore'
import { Input, Select, TextArea, Toggle } from '../../settings/public/controls'
import { dockApi } from './api'
import { PullRequestRow } from './PullRequestRow'
import { Collapse } from './PullRequestParts'
import type { PullRequestList } from '../../api/dockContracts'

export function PullRequestBranchCard({ list, workdir, lang, t, expanded, onToggle, onRefresh }: {
  list: PullRequestList
  workdir: string
  lang: Lang
  t: I18n
  expanded: boolean
  onToggle: () => void
  onRefresh: () => void
}) {
  const [creating, setCreating] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [base, setBase] = useState('')
  const [draft, setDraft] = useState(false)
  const [bases, setBases] = useState<string[]>([])
  const branch = list.currentBranch
  const current = list.current

  // 打开表单时：标题默认取最近一次提交说明，目标分支列出远端已有的分支。
  useEffect(() => {
    if (!creating) return
    let cancelled = false
    dockApi.gitLog(workdir, 1, 0, false).then((log) => {
      const subject = log.commits[0]?.subject ?? ''
      if (!cancelled && subject) setTitle((prev) => prev || subject)
    }).catch(() => {})
    dockApi.gitBranches(workdir).then((result) => {
      if (cancelled) return
      const names = [
        list.defaultBranch,
        ...result.branches.filter((item) => item.upstream).map((item) => item.name),
        ...result.remoteBranches.map((item) => item.name.replace(`${item.remote}/`, '')),
      ].filter((name) => name && name !== branch && name !== 'HEAD')
      setBases([...new Set(names)])
    }).catch(() => {})
    return () => { cancelled = true }
  }, [creating, workdir, branch, list.defaultBranch])

  if (!branch || (!current && branch === list.defaultBranch)) return null

  const target = base || list.defaultBranch
  const submit = async () => {
    if (!title.trim() || !target || submitting) return
    setSubmitting(true)
    try {
      await dockApi.pullRequestCreate(workdir, { title: title.trim(), body, base: target, draft })
      setCreating(false)
      setTitle('')
      setBody('')
      onRefresh()
    } catch (err) {
      notifyError(String(err))
    } finally {
      setSubmitting(false)
    }
  }

  const running = current?.checks === 'pending' && (current.state === 'open' || current.state === 'draft')
  const baseOptions = (bases.length ? bases : [list.defaultBranch])
    .filter(Boolean)
    .map((name) => ({ value: name, label: name }))

  return (
    <div className={`chat-pr-branch-card${running ? ' is-running' : ''}`}>
      <div className="chat-pr-branch-head">
        <GitBranch size={11} className="shrink-0" />
        <span className="shrink-0">{t.prCurrentBranch}</span>
        <span className="chat-pr-branch-name" title={branch}>{branch}</span>
      </div>
      {current ? (
        <div className="-mx-1.5 pb-1 pt-0.5">
          <PullRequestRow
            item={current}
            open={expanded}
            index={0}
            terms={[]}
            workdir={workdir}
            currentBranch={branch}
            lang={lang}
            t={t}
            onToggle={onToggle}
            onCheckedOut={onRefresh}
          />
        </div>
      ) : (
        <>
          <div className="chat-pr-branch-empty">
            <span className="min-w-0 flex-1">{t.prNoPrForBranch}</span>
            {!creating && (
              <Button size="sm" variant="primary" onClick={() => setCreating(true)}>
                <GitPullRequestCreate size={12} />
                {t.prCreate}
              </Button>
            )}
          </div>
          <Collapse open={creating}>
            <form
              className="chat-pr-create-form"
              onSubmit={(event) => {
                event.preventDefault()
                void submit()
              }}
            >
              <Input value={title} onChange={setTitle} placeholder={t.prCreateTitle} aria-label={t.prCreateTitle} />
              <TextArea value={body} onChange={setBody} placeholder={t.prCreateBody} rows={3} maxRows={8} aria-label={t.prCreateBody} />
              <div className="chat-pr-create-row">
                <span className="shrink-0">{t.prCreateBase}</span>
                <Select value={target} onChange={setBase} options={baseOptions} size="sm" className="min-w-0 flex-1" ariaLabel={t.prCreateBase} />
              </div>
              <div className="chat-pr-create-row">
                <Toggle checked={draft} onChange={setDraft} ariaLabel={t.prCreateDraft} />
                <span>{t.prCreateDraft}</span>
              </div>
              <div className="chat-pr-create-row">
                <span className="min-w-0 flex-1 text-[11px] text-[var(--text-faint)]">{t.prCreatePushHint}</span>
                <Button size="sm" variant="ghost" onClick={() => setCreating(false)} disabled={submitting}>{t.prCancel}</Button>
                <Button size="sm" variant="primary" type="submit" disabled={!title.trim() || !target || submitting}>
                  {submitting && <Loader2 size={12} className="animate-spin" />}
                  {submitting ? t.prCreating : t.prCreateSubmit}
                </Button>
              </div>
            </form>
          </Collapse>
        </>
      )}
    </div>
  )
}
