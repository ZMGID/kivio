import { useEffect, useState } from 'react'
import { api, providerAuthenticationReady, type Settings, type SubagentRoleSummary } from '../api/tauri'
import { peekSettings, updateSettingsCached } from '../api/settingsCache'
import { notifyError } from '../components/appNoticeStore'
import { ErrorNotice } from '../components/AppNotice'
import { Button } from '../components/Button'
import { useLang } from '../components/i18n'
import { Select, Toggle } from '../settings/public/controls'

type RoleChange =
  | { field: 'enabled'; value: boolean }
  | { field: 'model' | 'prewalk' | 'advisor'; value: string }

function configuredModels(settings: Settings) {
  return (settings.providers ?? []).flatMap(provider => {
    if (!provider.enabled || !providerAuthenticationReady(provider)) return []
    return provider.enabledModels.flatMap(model => {
      const value = `${provider.id}/${model}`
      return /[*?]/.test(value) || model.includes('@') ? [] : [{ value, label: `${model} · ${provider.name}`, title: value }]
    })
  })
}

function roleOverride<T>(overrides: Record<string, T> | undefined, name: string): T | undefined {
  return overrides && Object.hasOwn(overrides, name) ? overrides[name] : undefined
}

export function SubagentRoleSettings({ conversationId, settings, onSaved }: {
  conversationId?: string
  settings: Settings
  onSaved: (settings: Settings) => void
}) {
  const zh = useLang() === 'zh'
  const t = (cn: string, en: string) => zh ? cn : en
  const [roles, setRoles] = useState<SubagentRoleSummary[] | null>(null)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  const [saving, setSaving] = useState(false)
  useEffect(() => {
    let cancelled = false
    void api.chatSubagentRoles(conversationId).then(roles => {
      if (!cancelled) { setRoles(roles); setError('') }
    }, error => {
      if (!cancelled) setError(String(error))
    })
    return () => { cancelled = true }
  }, [conversationId, retry])

  const models = configuredModels(settings)
  const defaults = { value: '', label: t('按角色定义', 'Role default') }
  const modelOptions = [defaults, ...models]
  const policyOptions = [
    defaults,
    { value: 'on', label: t('启用 · 使用默认模型', 'On · default model') },
    { value: 'off', label: t('关闭', 'Off') },
    ...models,
  ]

  async function save(name: string, change: RoleChange) {
    setSaving(true)
    try {
      const saved = await updateSettingsCached(fresh => {
        const chatTools = { ...fresh.chatTools }
        if (change.field === 'enabled') {
          const disabled = new Set(chatTools.taskDisabledAgents)
          if (change.value) disabled.delete(name)
          else disabled.add(name)
          chatTools.taskDisabledAgents = [...disabled]
        } else {
          const value = change.value
          const selectable = configuredModels(fresh).some(option => option.value === value)
          const policySwitch = change.field !== 'model' && (value === 'on' || value === 'off')
          if (value && !selectable && !policySwitch) throw new Error(t('该模型已不可用，请重新选择。', 'This model is no longer available. Choose another model.'))
          switch (change.field) {
            case 'model': {
              const overrides = { ...chatTools.taskAgentModelOverrides }
              if (value) {
                const fallbacks = roleOverride(overrides, name)?.slice(1).filter(model => model !== value) ?? []
                chatTools.taskAgentModelOverrides = { ...overrides, [name]: [value, ...fallbacks] }
              } else {
                delete overrides[name]
                chatTools.taskAgentModelOverrides = overrides
              }
              break
            }
            case 'prewalk':
            case 'advisor': {
              const field = change.field === 'prewalk' ? 'taskAgentPrewalk' : 'taskAgentAdvisor'
              const overrides = { ...chatTools[field] }
              if (value) chatTools[field] = { ...overrides, [name]: value }
              else {
                delete overrides[name]
                chatTools[field] = overrides
              }
              break
            }
          }
        }
        return { ...fresh, chatTools }
      })
      onSaved(peekSettings() ?? saved)
    } catch (error) {
      notifyError(error instanceof Error ? error.message : String(error))
    } finally {
      setSaving(false)
    }
  }

  return <section aria-label={t('子代理角色', 'Subagent roles')} className="mt-5 space-y-3">
    <div>
      <h3 className="text-[13px] font-semibold text-[var(--text)]">{t('角色管理', 'Role management')}</h3>
      <p className="mt-1 text-xs text-[var(--text-muted)]">{t('设置应用于后续派工。已开始的任务可以完成。', 'Settings apply to future delegation. Active tasks can finish.')}</p>
    </div>
    {error ? <>
      <ErrorNotice message={error} />
      <Button size="sm" onClick={() => { setError(''); setRetry(count => count + 1) }}>{t('重试', 'Retry')}</Button>
    </> : !roles ? <p role="status" className="text-xs text-[var(--text-muted)]">{t('正在读取角色…', 'Loading roles…')}</p>
      : roles.length === 0 ? <p className="text-xs text-[var(--text-muted)]">{t('没有可管理的角色。', 'No roles found.')}</p>
        : <ul className="divide-y divide-[var(--color-border)] rounded-md border border-[var(--color-border)]">
          {roles.map(role => {
            const enabled = !settings.chatTools.taskDisabledAgents?.includes(role.name)
            const overrides = roleOverride(settings.chatTools.taskAgentModelOverrides, role.name) ?? []
            const fields = [
              { key: 'model', label: t('执行模型', 'Model'), value: overrides[0] ?? '', options: modelOptions },
              { key: 'prewalk', label: t('预先规划', 'Prewalk'), value: roleOverride(settings.chatTools.taskAgentPrewalk, role.name) ?? '', options: policyOptions },
              { key: 'advisor', label: t('审阅顾问', 'Advisor'), value: roleOverride(settings.chatTools.taskAgentAdvisor, role.name) ?? '', options: policyOptions },
            ] as const
            return <li key={role.id} className="space-y-3 px-4 py-3">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h4 className="break-words text-sm font-medium text-[var(--text)]">{role.name} <span className="text-xs font-normal text-[var(--text-muted)]">{role.source}</span></h4>
                  {role.description && <p className="mt-1 break-words text-xs text-[var(--text-muted)]">{role.description}</p>}
                  {role.unsupported.length > 0 && <p className="mt-1 text-xs text-[var(--danger)]">{t('当前不支持', 'Unsupported')}: {role.unsupported.join(', ')}</p>}
                </div>
                <Toggle checked={enabled} disabled={saving} ariaLabel={t(`启用 ${role.name}`, `Enable ${role.name}`)}
                  onChange={value => void save(role.name, { field: 'enabled', value })} />
              </div>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(155px,1fr))] gap-3">
                {fields.map(field => <div key={field.key} className="min-w-0 space-y-1">
                  <p className="text-xs text-[var(--text-muted)]">{field.label}</p>
                  <Select size="sm" ariaLabel={`${role.name} · ${field.label}`} value={field.value} disabled={saving || !enabled}
                    options={field.options.some(option => option.value === field.value)
                      ? field.options : [...field.options, { value: field.value, label: field.value }]}
                    onChange={value => { if (value !== field.value) void save(role.name, { field: field.key, value }) }} />
                </div>)}
              </div>
              {overrides.length > 1 && <p className="text-xs text-[var(--text-muted)]">{t('备用模型', 'Fallback models')} · {overrides.slice(1).join(', ')}</p>}
            </li>
          })}
        </ul>}
  </section>
}
