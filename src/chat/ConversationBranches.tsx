import { useEffect, useRef, useState } from 'react'
import { Button } from '../components/Button'
import { notifyError } from '../components/appNoticeStore'
import type { Lang } from '../components/i18n'
import { Select } from '../settings/public/controls'
import { chatApi, type ConversationBranch } from './api'

/** 历史分支对话框：入口在标题栏工作区面板。打开时读取，确认切换成功后才关闭。 */
export function ConversationBranchDialog({ conversationId, disabled, lang, onSwitch, onClose }: {
  conversationId: string
  disabled: boolean
  lang: Lang
  onSwitch: (branchId: string) => Promise<boolean>
  onClose: () => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [branches, setBranches] = useState<ConversationBranch[] | null>(null)
  const [selectedId, setSelectedId] = useState('')
  const [switching, setSwitching] = useState(false)
  const [failed, setFailed] = useState(false)
  const zh = lang === 'zh'
  useEffect(() => {
    dialog.current?.showModal()
    let cancelled = false
    void chatApi.getBranches(conversationId).then(result => {
      if (cancelled) return
      setBranches(result)
      setSelectedId(result.find(branch => branch.active)?.id ?? '')
    }).catch(error => {
      if (cancelled) return
      setFailed(true)
      notifyError(error instanceof Error ? error.message : String(error))
    })
    return () => { cancelled = true }
  }, [conversationId])
  const active = branches?.find(branch => branch.active)?.id
  return <dialog ref={dialog} className="kv-modal" aria-label={zh ? '历史分支' : 'History branches'}
    onCancel={event => { event.preventDefault(); if (!switching) onClose() }}>
    <form className="flex flex-col gap-4" onSubmit={event => {
      event.preventDefault()
      if (disabled || switching || !selectedId || selectedId === active) return
      setSwitching(true)
      void onSwitch(selectedId).then(applied => {
        if (applied) onClose()
      }).finally(() => setSwitching(false))
    }}>
      <h2>{zh ? '历史分支' : 'History branches'}</h2>
      {branches?.length ? <Select ariaLabel={zh ? '选择分支' : 'Choose branch'} value={selectedId}
        onChange={setSelectedId} disabled={disabled || switching}
        options={branches.map(branch => ({ value: branch.id, label: branch.active
          ? `${branch.label} (${zh ? '当前' : 'current'})` : branch.label }))} />
        : <p role="status">{failed ? (zh ? '读取失败，请关闭后重试。' : 'Loading failed. Close and retry.')
          : branches ? (zh ? '暂无历史分支' : 'No history branches') : (zh ? '正在读取…' : 'Loading…')}</p>}
      <footer className="flex justify-end gap-2">
        <Button variant="ghost" disabled={switching} onClick={onClose}>{zh ? '关闭' : 'Close'}</Button>
        <Button type="submit" variant="primary" disabled={disabled || switching || !selectedId || selectedId === active}>
          {switching ? (zh ? '正在切换…' : 'Switching…') : (zh ? '切换分支' : 'Switch branch')}
        </Button>
      </footer>
    </form>
  </dialog>
}
