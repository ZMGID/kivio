import { useEffect, useRef, useState } from 'react'
import { api } from '../../api/tauri'
import { getSettingsCached } from '../../api/settingsCache'
import { Button } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useT } from '../../components/i18n'
import { Select, TextArea, Toggle } from '../../settings/public/controls'
import { ModelSelector } from '../ModelSelector'
import { ThinkingLevelSelector } from '../ThinkingLevelSelector'
import type { ThinkingLevel } from '../types'
import { isTauriRuntime } from '../utils'
import {
  isProfileConflict,
  profileApi,
  type ApprovalSource,
  type BotMemoryView,
  type BotProfileDetail,
  type MemoryLayer,
} from './profile'

const APPROVAL_POLICIES = ['always_confirm', 'readonly_auto_sensitive_confirm', 'auto'] as const

function approvalLabel(policy: string, t: ReturnType<typeof useT>) {
  if (policy === 'always_confirm') return t.chatBotApprovalAlways
  if (policy === 'auto') return t.chatBotApprovalAuto
  if (policy === 'readonly_auto_sensitive_confirm') return t.chatBotApprovalStandard
  return policy
}

export type BotThinkingSettings = {
  value: ThinkingLevel | null
  onChange: (level: ThinkingLevel) => void
}

export function BotRuntimePanel({ assistantId, onActivity, thinkingSettings, section = 'runtime' }: {
  assistantId: string
  section?: 'profile' | 'runtime' | 'tools' | 'memory'
  /** Editor leave guard. Report the panel's own dirty/busy; do not keep a second guard store. */
  onActivity?: (activity: { dirty: boolean; busy: boolean }) => void
  thinkingSettings?: BotThinkingSettings
}) {
  const t = useT()
  const [profile, setProfile] = useState<BotProfileDetail | null>(null)
  const [memory, setMemory] = useState<BotMemoryView | null>(null)
  const [l1, setL1] = useState('')
  const [l2, setL2] = useState('')
  const [sharedDraft, setSharedDraft] = useState('')
  const [mcpIds, setMcpIds] = useState<string[]>([])
  const [skillIds, setSkillIds] = useState<string[]>([])
  const [mcpChoices, setMcpChoices] = useState<Array<{ id: string; name: string }>>([])
  const [skillChoices, setSkillChoices] = useState<Array<{ id: string; name: string }>>([])
  const [editingGrants, setEditingGrants] = useState(false)
  const [error, setError] = useState('')
  const [conflict, setConflict] = useState(false)
  const [stale, setStale] = useState(false)
  const [busy, setBusy] = useState(false)
  const dirty = useRef(false)
  useEffect(() => {
    if (!assistantId || !isTauriRuntime()) return
    let current = true
    setError('')
    void profileApi.get(assistantId).then((next) => {
      if (!current) return
      setProfile(next)
      setMcpIds(next.resolved.mcpServerIds)
      setSkillIds(next.resolved.skillIds)
    }).catch((err) => {
      if (current) setError(err instanceof Error ? err.message : String(err))
    })
    void profileApi.memory(assistantId).then((next) => {
      if (!current) return
      setMemory(next)
      setL1(next.l1 ?? '')
      setL2(next.l2 ?? '')
    }).catch((err) => {
      if (current) setError(err instanceof Error ? err.message : String(err))
    })
    void getSettingsCached().then((settings) => {
      if (!current) return
      setMcpChoices((settings.chatTools?.servers ?? []).map((server) => ({ id: server.id, name: server.name || server.id })))
    }).catch(() => { if (current) setMcpChoices([]) })
    void api.chatSkillsList().then((result) => {
      if (!current) return
      setSkillChoices((result.skills ?? []).map((skill) => ({ id: skill.id, name: skill.name || skill.id })))
    }).catch(() => { if (current) setSkillChoices([]) })
    return () => { current = false }
  }, [assistantId])
  useEffect(() => {
    let stop = () => {}
    let current = true
    void profileApi.onChanged(() => {
      if (!current || dirty.current) {
        if (current && dirty.current) setStale(true)
        return
      }
      void profileApi.get(assistantId).then((next) => { if (current && !dirty.current) setProfile(next) }).catch((err) => {
        if (current) setError(err instanceof Error ? err.message : String(err))
      })
      void profileApi.memory(assistantId).then((next) => {
        if (!current || dirty.current) return
        setMemory(next)
        setL1(next.l1 ?? '')
        setL2(next.l2 ?? '')
      }).catch((err) => {
        if (current) setError(err instanceof Error ? err.message : String(err))
      })
    }).then((unlisten) => {
      if (!current) unlisten()
      else stop = unlisten
    })
    return () => { current = false; stop() }
  }, [assistantId])
  const runtimeDirty = editingGrants || sharedDraft.trim() !== '' || (memory != null && (l1 !== (memory.l1 ?? '') || l2 !== (memory.l2 ?? '')))
  dirty.current = runtimeDirty
  useEffect(() => {
    onActivity?.({ dirty: runtimeDirty, busy })
    return () => onActivity?.({ dirty: false, busy: false })
  }, [onActivity, runtimeDirty, busy])
  if (!isTauriRuntime() || !assistantId) return null
  const run = async (action: () => Promise<void>) => {
    if (busy) return
    setBusy(true)
    setError('')
    try {
      await action()
      setConflict(false)
      setStale(false)
    } catch (err) {
      setConflict(isProfileConflict(err))
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }
  const applyProfile = (next: BotProfileDetail) => {
    setProfile(next)
    setMcpIds(next.resolved.mcpServerIds)
    setSkillIds(next.resolved.skillIds)
    setEditingGrants(false)
  }
  const reload = () => {
    const pull = () => void run(async () => {
      const [nextProfile, nextMemory] = await Promise.all([
        profileApi.get(assistantId),
        profileApi.memory(assistantId),
      ])
      applyProfile(nextProfile)
      setMemory(nextMemory)
      setL1(nextMemory.l1 ?? '')
      setL2(nextMemory.l2 ?? '')
      setSharedDraft('')
    })
    if (!dirty.current) {
      pull()
      return
    }
    void confirmDialog({ message: t.chatBotReloadConfirm, confirmLabel: t.chatBotReloadProfile }).then((yes) => {
      if (yes) pull()
    })
  }
  const commitLayer = (layer: MemoryLayer, next: BotMemoryView) => {
    setMemory((current) => current ? {
      ...next,
      l1: layer === 'l1' ? next.l1 : current.l1,
      l2: layer === 'l2' ? next.l2 : current.l2,
    } : next)
    if (layer === 'l1') setL1(next.l1 ?? '')
    else setL2(next.l2 ?? '')
  }
  const saveLayer = (layer: MemoryLayer, content: string) => {
    void run(async () => {
      commitLayer(layer, await profileApi.saveMemory(assistantId, layer, content))
    })
  }
  const clearLayer = (layer: MemoryLayer) => {
    void confirmDialog({ message: t.chatBotMemoryClearConfirm, confirmLabel: t.chatBotMemoryClear, danger: true }).then((yes) => {
      if (!yes) return
      void run(async () => {
        commitLayer(layer, await profileApi.clearMemory(assistantId, layer))
      })
    })
  }
  const toggle = (list: string[], id: string, set: (ids: string[]) => void, checked: boolean) => {
    set(checked ? [...list, id] : list.filter((item) => item !== id))
  }
  return (
    <div hidden={section === 'profile'}>
    <div className="flex w-full flex-col gap-4 text-left">
      {error && <p className="text-[12px] text-red-600" role="alert">{error}</p>}
      {(conflict || stale) && <Button size="sm" disabled={busy} onClick={reload}>{t.chatBotReloadProfile}</Button>}
      {!profile && !error && <p className="text-[12px] text-neutral-500" aria-busy="true">{t.chatLoading}</p>}
      {profile && (
        <>
          <section hidden={section !== 'runtime'} aria-label={t.chatBotRunSettings}>
            <h3 className="mb-4 text-[14px] font-semibold text-neutral-900">{t.chatBotRunSettings}</h3>
            <div className="chat-bot-setting-row">
              <span>{t.chatBotModelSource}</span>
              <fieldset disabled={busy} className="min-w-0 justify-self-end">
                <ModelSelector currentProviderId={profile.resolvedProviderId} currentModel={profile.resolvedModel} onModelChange={(providerId, model) => {
                  void run(async () => applyProfile(await profileApi.updateModel(assistantId, profile.profile.revision, 'explicit', providerId, model)))
                }} />
              </fieldset>
            </div>
            <div className="flex justify-end">
              {profile.modelSource === 'inherit_global'
                ? <p className="text-[11px] text-neutral-500">{t.chatBotModelInherit}</p>
                : <Button size="sm" variant="ghost" disabled={busy} onClick={() => void run(async () => {
                  applyProfile(await profileApi.updateModel(assistantId, profile.profile.revision, 'inherit_global'))
                })}>{t.chatBotResetModel}</Button>}
            </div>
            {thinkingSettings && <div className="chat-bot-setting-row">
              <span>{t.chatBotThinking}</span>
              <fieldset disabled={busy} className="min-w-0 justify-self-end">
                <ThinkingLevelSelector currentProviderId={profile.resolvedProviderId} currentModel={profile.resolvedModel}
                  value={thinkingSettings.value} onChange={thinkingSettings.onChange} />
              </fieldset>
            </div>}
            <div className="chat-bot-setting-row">
              <span>{t.chatBotApprovalSource}</span>
              <div>
                <Select ariaLabel={t.chatBotApprovalSource} value={profile.profile.approval.source === 'override' ? (profile.profile.approval.policy ?? '') : 'inherit_global'} disabled={busy} onChange={(value) => {
                  void run(async () => {
                    const source: ApprovalSource = value === 'inherit_global' ? 'inherit_global' : 'override'
                    applyProfile(await profileApi.updateApproval(assistantId, profile.profile.revision, source, source === 'override' ? value : ''))
                  })
                }} options={[
                  { value: 'inherit_global', label: t.chatBotApprovalInherit },
                  ...APPROVAL_POLICIES.map((policy) => ({ value: policy, label: approvalLabel(policy, t) })),
                ]} />
              </div>
            </div>
            <div className="chat-bot-setting-row">
              <span>{t.chatBotPrimary}</span>
              <div><Toggle ariaLabel={t.chatBotSetPrimary} checked={profile.profile.primary} disabled={busy} onChange={value => void run(async () => {
                await profileApi.setPrimary(assistantId, value)
                applyProfile(await profileApi.get(assistantId))
              })} /></div>
            </div>
          </section>
          <section hidden={section !== 'tools'}>
            <h3 className="text-[14px] font-semibold text-neutral-900">{t.chatBotCapability}</h3>
            <p className="mt-1 text-[12px] text-neutral-500">{t.chatBotSkills} {profile.resolved.skillIds.length} · {t.chatBotMcp} {profile.resolved.mcpServerIds.length}</p>
            {editingGrants && (
              <div className="mt-2 flex flex-col gap-2">
                <fieldset className="flex max-h-36 flex-col gap-1 overflow-y-auto custom-scrollbar">
                  <legend className="text-[12px] text-neutral-500">{t.chatBotMcp}</legend>
                  {mcpChoices.length === 0 && <p className="text-[12px] text-neutral-500">{t.chatBotAllowlistEmpty}</p>}
                  {mcpChoices.map((choice) => (
                    <label key={choice.id} className="flex items-center gap-2 text-[12px] text-neutral-800">
                      <input type="checkbox" checked={mcpIds.includes(choice.id)} onChange={(event) => toggle(mcpIds, choice.id, setMcpIds, event.target.checked)} />
                      {choice.name}
                    </label>
                  ))}
                </fieldset>
                <fieldset className="flex max-h-36 flex-col gap-1 overflow-y-auto custom-scrollbar">
                  <legend className="text-[12px] text-neutral-500">{t.chatBotSkills}</legend>
                  {skillChoices.length === 0 && <p className="text-[12px] text-neutral-500">{t.chatBotAllowlistEmpty}</p>}
                  {skillChoices.map((choice) => (
                    <label key={choice.id} className="flex items-center gap-2 text-[12px] text-neutral-800">
                      <input type="checkbox" checked={skillIds.includes(choice.id)} onChange={(event) => toggle(skillIds, choice.id, setSkillIds, event.target.checked)} />
                      {choice.name}
                    </label>
                  ))}
                </fieldset>
              </div>
            )}
            <div className="mt-2 flex flex-wrap gap-2">
              <Button size="sm" disabled={busy} onClick={() => void run(async () => {
                applyProfile(await profileApi.updateCapability(assistantId, profile.profile.revision, 'granted_defaults'))
              })}>{t.chatBotUseGranted}</Button>
              <Button size="sm" disabled={busy} onClick={() => {
                if (!editingGrants) {
                  setMcpIds(profile.resolved.mcpServerIds)
                  setSkillIds(profile.resolved.skillIds)
                  setEditingGrants(true)
                  return
                }
                const empty = mcpIds.length === 0 && skillIds.length === 0
                const save = () => void run(async () => {
                  applyProfile(await profileApi.updateCapability(assistantId, profile.profile.revision, 'explicit', mcpIds, skillIds))
                })
                if (!empty) { save(); return }
                void confirmDialog({ message: t.chatBotExplicitEmptyConfirm, confirmLabel: t.chatBotUseExplicit, danger: true }).then((yes) => {
                  if (yes) save()
                })
              }}>{editingGrants ? t.chatBotSaveExplicit : t.chatBotUseExplicit}</Button>
            </div>
          </section>

        </>
      )}
      <section hidden={section !== 'memory'}>
        <h3 className="text-[14px] font-semibold text-neutral-900">{t.chatBotMemoryPrivate}</h3>
        {memory && memory.globalInjection !== 'disabled' && <p className="mt-1 text-[12px] text-neutral-600">{memory.globalInjection}</p>}
        {memory && (
          <>
            <label className="mt-3 flex flex-col gap-1.5 text-[12px] text-neutral-600">
              {t.chatBotMemoryL1}
              <TextArea rows={4} value={l1} disabled={busy} onChange={setL1} />
            </label>
            <div className="mt-2 flex gap-2">
              <Button size="sm" disabled={busy} onClick={() => saveLayer('l1', l1)}>{t.chatBotMemorySave}</Button>
              <Button size="sm" disabled={busy} onClick={() => clearLayer('l1')}>{t.chatBotMemoryClear}</Button>
            </div>
            <label className="mt-3 flex flex-col gap-1.5 text-[12px] text-neutral-600">
              {t.chatBotMemoryL2}
              <TextArea rows={4} value={l2} disabled={busy} onChange={setL2} />
            </label>
            <div className="mt-2 flex gap-2">
              <Button size="sm" disabled={busy} onClick={() => saveLayer('l2', l2)}>{t.chatBotMemorySave}</Button>
              <Button size="sm" disabled={busy} onClick={() => clearLayer('l2')}>{t.chatBotMemoryClear}</Button>
            </div>
            <h4 className="mt-4 text-[12px] font-semibold text-neutral-800">{t.chatBotSharedMemory}</h4>
            <p className="mt-1 text-[12px] leading-5 text-neutral-500">{t.chatBotSharedHint}</p>
            {memory.sharedMemory.length === 0 && <p className="mt-1 text-[12px] text-neutral-500">{t.chatBotSharedEmpty}</p>}
            <ul className="mt-2 flex flex-col gap-2">
              {memory.sharedMemory.map((note) => (
                <li key={note.id} className="rounded-lg border border-neutral-200/80 px-3 py-2">
                  <p className="whitespace-pre-wrap text-[13px] leading-5 text-neutral-800">{note.content}</p>
                  <div className="mt-2">
                    <Button size="sm" disabled={busy} onClick={() => {
                      void confirmDialog({ message: t.chatBotSharedDeleteConfirm, confirmLabel: t.chatBotSharedDelete, danger: true }).then((yes) => {
                        if (!yes) return
                        void run(async () => {
                          await profileApi.deleteShared(assistantId, note.id)
                          const next = await profileApi.memory(assistantId)
                          setMemory(next)
                        })
                      })
                    }}>{t.chatBotSharedDelete}</Button>
                  </div>
                </li>
              ))}
            </ul>
            <label className="mt-2 flex flex-col gap-1.5 text-[12px] text-neutral-600">
              {t.chatBotSharedAdd}
              <TextArea rows={3} value={sharedDraft} disabled={busy} onChange={setSharedDraft} />
            </label>
            <div className="mt-2">
              <Button size="sm" disabled={busy || !sharedDraft.trim()} onClick={() => void run(async () => {
                await profileApi.addShared(assistantId, sharedDraft.trim())
                setSharedDraft('')
                const next = await profileApi.memory(assistantId)
                setMemory(next)
              })}>{t.chatBotSharedAdd}</Button>
            </div>
          </>
        )}
      </section>
    </div>
    </div>
  )
}
