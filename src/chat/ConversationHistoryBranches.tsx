import { useEffect, useState } from 'react'
import { Check, GitFork, Loader2 } from 'lucide-react'
import { notifyError } from '../components/appNoticeStore'
import { i18n, type Lang } from '../components/i18n'
import { chatApi, type ConversationBranch } from './api'
import { PANEL_ICON, PanelSection, SplitRow } from './dock/WorkspacePanelParts'

export type HistoryBranchActions = {
  disabled: boolean
  revision?: number
  onSwitch: (branchId: string) => Promise<boolean>
}

type Props = HistoryBranchActions & {
  conversationId: string
  lang: Lang
  active?: boolean
  presentation?: 'panel' | 'toolbar'
}

export function ConversationHistoryBranches(props: Props) {
  return <HistoryBranchPicker key={props.conversationId} {...props} />
}

function HistoryBranchPicker({ conversationId, lang, disabled, onSwitch, revision, active = true, presentation = 'toolbar' }: Props) {
  const t = i18n[lang]
  const [branches, setBranches] = useState<ConversationBranch[] | null>(null)
  const [switching, setSwitching] = useState(false)
  useEffect(() => {
    setBranches(null)
    if (!active) return
    let cancelled = false
    chatApi.getBranches(conversationId)
      .then(list => { if (!cancelled) setBranches(list) })
      .catch(error => { if (!cancelled) notifyError(error instanceof Error ? error.message : String(error)) })
    return () => { cancelled = true }
  }, [active, conversationId, revision])
  const switchBranch = async (branch: ConversationBranch) => {
    if (disabled || switching || branch.active) return
    setSwitching(true)
    try {
      if (await onSwitch(branch.id)) setBranches(await chatApi.getBranches(conversationId))
    } catch (error) {
      notifyError(error instanceof Error ? error.message : String(error))
    } finally {
      setSwitching(false)
    }
  }
  if (!branches || branches.length < 2) return null
  const picker = <SplitRow
    icon={switching ? <Loader2 className={`${PANEL_ICON} animate-spin`} /> : <GitFork className={PANEL_ICON} />}
    label={branches.find(branch => branch.active)?.label ?? t.workspaceHistoryBranches}
    meta={t.workspaceHistoryCount.replace('{n}', String(branches.length))}
    disabled={disabled || switching}
    menuLabel={t.workspaceHistoryBranches}
    menu={close => branches.map(branch => <button key={branch.id} type="button" role="menuitemradio"
      aria-checked={branch.active} disabled={branch.active || disabled || switching}
      className={`kv-menu-row w-full text-neutral-800 transition-colors hover:bg-neutral-100 disabled:opacity-50 ${branch.active ? 'font-medium disabled:opacity-100' : ''}`}
      onClick={() => { close(); void switchBranch(branch) }}>
      <span className="min-w-0 flex-1 truncate">{branch.label}</span>
      {branch.active && <Check size={13} strokeWidth={2} className="shrink-0 text-neutral-500" />}
    </button>)}
  />
  return presentation === 'panel' ? <PanelSection>{picker}</PanelSection> : <div className="max-w-48" data-tauri-drag-region="false">{picker}</div>
}
