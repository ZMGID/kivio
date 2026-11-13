// 「发布仓库」：没有远程时在面板里就地填写仓库名与可见性，用 GitHub CLI 创建并推送。
// 参照 T3 的 PublishRepositoryDialog，但不弹窗，表单直接展开在 Git 段落下方。
import { useEffect, useState } from 'react'
import { Button } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { Input } from '../../settings/public/controls'
import { dockApi } from './api'

export function WorkspacePublishForm({ workdir, defaultName, lang, busy, onPublish, onCancel }: {
  workdir: string
  defaultName: string
  lang: Lang
  busy: boolean
  onPublish: (repository: string, isPrivate: boolean) => void
  onCancel: () => void
}) {
  const t = i18n[lang]
  const [readiness, setReadiness] = useState<{ ready: boolean; account: string; hint: string } | null>(null)
  const [name, setName] = useState<string | null>(null)
  const [isPrivate, setIsPrivate] = useState(true)

  useEffect(() => {
    let cancelled = false
    dockApi.gitPublishReadiness(workdir)
      .then((result) => { if (!cancelled) setReadiness(result) })
      .catch(() => { if (!cancelled) setReadiness({ ready: false, account: '', hint: t.workspacePublishUnavailable }) })
    return () => { cancelled = true }
  }, [workdir, t.workspacePublishUnavailable])

  const value = name ?? (readiness?.account ? `${readiness.account}/${defaultName}` : defaultName)
  const valid = /^[A-Za-z0-9_.-]+(\/[A-Za-z0-9_.-]+)?$/.test(value.trim())

  if (readiness === null) {
    return <div role="status" className="px-2.5 py-2 text-[12px] text-neutral-400">{t.dockLoading}</div>
  }
  if (!readiness.ready) {
    return (
      <div className="flex items-start justify-between gap-2 px-2.5 py-2 text-[12px] text-neutral-500">
        <span className="min-w-0">{readiness.hint || t.workspacePublishUnavailable}</span>
        <Button size="sm" variant="ghost" onClick={onCancel}>{t.workspacePublishCancel}</Button>
      </div>
    )
  }
  return (
    <form
      className="chat-motion-fade-up flex flex-col gap-2 px-2.5 pb-2 pt-1"
      onSubmit={(event) => {
        event.preventDefault()
        if (valid && !busy) onPublish(value.trim(), isPrivate)
      }}
    >
      <Input
        aria-label={t.workspacePublishName}
        placeholder={t.workspacePublishName}
        value={value}
        onChange={setName}
        autoFocus
      />
      <div role="radiogroup" aria-label={t.workspacePublishVisibility} className="flex gap-1 text-[12px]">
        {[true, false].map((option) => (
          <button
            key={String(option)}
            type="button"
            role="radio"
            aria-checked={isPrivate === option}
            onClick={() => setIsPrivate(option)}
            className={`flex-1 rounded-md px-2 py-1 transition-colors ${isPrivate === option
              ? 'bg-black/[0.06] font-medium text-neutral-800 dark:bg-white/[0.1] dark:text-neutral-100'
              : 'text-neutral-500 hover:bg-black/[0.035] dark:hover:bg-white/[0.06]'}`}
          >
            {option ? t.workspacePublishPrivate : t.workspacePublishPublic}
          </button>
        ))}
      </div>
      <div className="flex justify-end gap-1.5">
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy}>{t.workspacePublishCancel}</Button>
        <Button size="sm" variant="primary" type="submit" disabled={!valid || busy}>
          {busy ? t.workspacePublishing : t.workspacePublish}
        </Button>
      </div>
    </form>
  )
}
