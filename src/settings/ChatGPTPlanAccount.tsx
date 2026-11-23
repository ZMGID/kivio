import { useEffect, useRef, useState } from 'react'
import { api, CHATGPT_USAGE_URL, type ChatGPTAccount, type ModelProvider, type Settings } from '../api/tauri'
import { Button } from '../components/Button'
import { ErrorNotice } from '../components/AppNotice'
import type { Lang } from '../components/i18n'
import { Select } from './components'

function accountOptions(accounts: ChatGPTAccount[], zh: boolean) {
  return accounts.map(account => {
    const name = account.email || account.subject
    const duplicate = accounts.some(other => other.credentialId !== account.credentialId && (other.email || other.subject) === name)
    if (name && !duplicate) return { value: account.credentialId, label: name }
    const suffix = account.credentialId.slice(-8)
    const id = accounts.some(other => other.credentialId !== account.credentialId && other.credentialId.endsWith(suffix))
      ? account.credentialId : suffix
    return { value: account.credentialId, label: `${name || (zh ? '待完成授权' : 'Pending sign-in')} · ${id}` }
  })
}

/** A saved app registration can outlive sign-out or have identity access only. */
export function ChatGPTPlanAccount({ provider, settings, lang, busy, accountListRevision, onSignIn, onDisconnect, onFetchModels, onUpdateProvider }: {
  provider: ModelProvider
  settings: Settings
  lang: Lang
  busy: boolean
  accountListRevision: number
  onSignIn: (credentialId?: string) => void
  onDisconnect: () => void
  onFetchModels: (id: string) => Promise<boolean>
  onUpdateProvider: (id: string, updates: Partial<ModelProvider>) => void
}) {
  const zh = lang === 'zh'
  const auth = provider.request.oauth
  const credentialId = auth?.credentialId
  const [accounts, setAccounts] = useState<ChatGPTAccount[] | null>(null)
  const [selection, setSelection] = useState(credentialId)
  const [refresh, setRefresh] = useState(0)
  const [error, setError] = useState('')
  const [catalogChecked, setCatalogChecked] = useState(false)
  const current = useRef({ provider, onUpdateProvider })
  current.current = { provider, onUpdateProvider }

  useEffect(() => {
    let active = true
    setError('')
    void api.providerOAuthChatGPTAccounts().then(next => {
      if (!active) return
      setAccounts(next)
      const { provider: latest, onUpdateProvider: update } = current.current
      const latestAuth = latest.request.oauth
      if (!credentialId || !latestAuth || latestAuth.credentialId !== credentialId) return
      const account = next.find(item => item.credentialId === credentialId)
      const needsGrant = !account?.connected || !account.planEnabled
      if (needsGrant !== Boolean(latestAuth.requiresReauthorization)) {
        update(latest.id, { request: { ...latest.request, oauth: { ...latestAuth, requiresReauthorization: needsGrant } } })
      }
    }).catch(err => { if (active) setError(String(err)) })
    return () => { active = false }
  }, [credentialId, refresh, accountListRevision])

  const account = accounts?.find(item => item.credentialId === credentialId)
  const options = accountOptions(accounts ?? [], zh)
  const selectedAccount = selection === '' ? undefined : accounts?.find(item => item.credentialId === selection) ?? accounts?.[0]
  const selected = selectedAccount?.credentialId ?? ''
  const ready = Boolean(account?.connected && account.planEnabled && !auth?.requiresReauthorization)
  useEffect(() => {
    let active = true
    setCatalogChecked(false)
    if (ready) void onFetchModels(provider.id).then(applied => { if (active) setCatalogChecked(applied) })
    return () => { active = false }
  }, [credentialId, ready, onFetchModels, provider.id])

  const bindings = [
    ...Object.values(settings.defaultModels), settings.lens, settings.screenshotTranslation,
    { providerId: settings.chatProviderId, model: settings.chatModel },
    { providerId: settings.translatorProviderId, model: settings.translatorModel },
    { providerId: settings.voice?.orchestratorProviderId, model: settings.voice?.orchestratorModel },
  ]
  const configured = new Set([...provider.enabledModels, ...bindings.filter(binding => binding?.providerId === provider.id).map(binding => binding.model)])
  const unavailable = catalogChecked ? [...configured].filter(model => model && !provider.availableModels.includes(model)) : []
  const status = account?.connected && !account.planEnabled
    ? (zh ? '已登录。使用模型还需要授权 ChatGPT 套餐。' : 'Signed in. Grant access to your ChatGPT plan to use models.')
    : credentialId && accounts && !account?.connected
      ? (zh ? '账号已断开。请重新登录。' : 'Account disconnected. Sign in again.')
      : ''
  return <div className="space-y-2">
    {account?.connected && <p className="text-sm" role="status">{zh ? '当前账号' : 'Current account'} · {options.find(option => option.value === credentialId)?.label}</p>}
    {status && <p className="text-sm" role="status">{status}</p>}
    {accounts && accounts.length > 0 && <Select
      ariaLabel={zh ? '登录账号' : 'Sign-in account'} value={selected} disabled={busy} onChange={setSelection}
      options={[...options, { value: '', label: zh ? '使用其他 ChatGPT 账号' : 'Use another ChatGPT account' }]}
    />}
    <div className="flex flex-wrap items-center gap-2">
      <Button size="sm" disabled={busy || !accounts} onClick={() => onSignIn(selected || undefined)}>
        {selectedAccount?.subject === null ? (zh ? '继续授权' : 'Continue sign-in')
          : account?.connected && !account.planEnabled && selected === credentialId
          ? (zh ? '授权 ChatGPT 套餐' : 'Authorize ChatGPT plan')
          : ready && selected === credentialId ? (zh ? '重新登录' : 'Sign in again') : (zh ? '使用 ChatGPT 登录' : 'Sign in with ChatGPT')}
      </Button>
      {account?.connected && <Button size="sm" disabled={busy} onClick={onDisconnect}>{zh ? '断开授权' : 'Disconnect'}</Button>}
      {account?.connected && <Button variant="ghost" size="sm" onClick={() => void api.openExternal(CHATGPT_USAGE_URL).catch(err => setError(String(err)))}>
        {zh ? '管理 ChatGPT 用量 ↗' : 'Manage ChatGPT usage ↗'}
      </Button>}
      {error && <Button size="sm" onClick={() => setRefresh(n => n + 1)}>{zh ? '重试' : 'Retry'}</Button>}
    </div>
    {unavailable.length > 0 && <p className="text-sm" role="status">
      {zh ? `当前账号无法使用 ${unavailable.join('、')}。请在模型列表中选择其他模型，并更新原有选择。`
        : `This account cannot use ${unavailable.join(', ')}. Choose an available model and update your selection.`}
    </p>}
    {catalogChecked && provider.availableModels.length === 0 && <p className="text-sm" role="status">{zh ? '当前账号没有可用模型。' : 'No models are available for this account.'}</p>}
    {error && <ErrorNotice message={error} />}
  </div>
}
