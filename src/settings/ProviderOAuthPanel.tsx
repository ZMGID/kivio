import { ErrorNotice } from '../components/AppNotice'
import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type ModelProvider, type ProviderOAuthLogin, type Settings } from '../api/tauri'
import { Button } from '../components/Button'
import { FieldBlock, Select } from './components'
import type { Lang } from '../components/i18n'
import { ChatGPTPlanAccount } from './ChatGPTPlanAccount'
import { PROVIDER_PRESETS } from './providerPresets'
import { ProviderAccountIdentity } from './ProviderAccountIdentity'

export function ProviderOAuthPanel({ provider, settings, lang, onUpdateProvider, onFetchModels, authorizedNotice }: {
  provider: ModelProvider
  settings: Settings
  onFetchModels: (id: string) => Promise<boolean>
  lang: Lang
  onUpdateProvider: (id: string, updates: Partial<ModelProvider>) => void
  authorizedNotice?: string
}) {
  const zh = lang === 'zh'
  const auth = provider.request.oauth
  const chatgpt = auth?.provider === 'chatgpt'
  const [login, setLogin] = useState<ProviderOAuthLogin | null>(null)
  const [operation, setOperation] = useState<'sign-in' | 'disconnect' | null>(null)
  const busy = operation !== null
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [accountRevision, setAccountRevision] = useState(0)
  const [accountListRevision, setAccountListRevision] = useState(0)
  const generation = useRef(0)
  const loginRef = useRef<ProviderOAuthLogin | null>(null)
  const current = useRef({ provider, onUpdateProvider })
  current.current = { provider, onUpdateProvider }

  useEffect(() => () => {
    generation.current += 1
    if (loginRef.current) void api.providerOAuthCancel(loginRef.current.loginId).catch(() => {})
  }, [])

  const cancelLogin = useCallback(async (loginId: string) => {
    await api.providerOAuthCancel(loginId).catch(() => {})
    // The callback may have persisted a new client before the token exchange stopped.
    if (chatgpt) setAccountListRevision(value => value + 1)
  }, [chatgpt])

  useEffect(() => {
    if (!login) return
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      try {
        const result = await api.providerOAuthPoll(login.loginId)
        if (stopped) {
          if (result.auth?.credentialId && result.auth.provider !== 'chatgpt') void api.providerOAuthDisconnect(result.auth.credentialId).catch(() => {})
          return
        }
        if (result.status === 'authorized' && result.auth) {
          const { provider: latest, onUpdateProvider: update } = current.current
          const request = { ...latest.request, oauth: result.auth }
          const plan = result.auth.provider === 'chatgpt'
          if (plan) setAccountRevision(value => value + 1)
          update(latest.id, plan ? { request, availableModels: [] } : { request })
          loginRef.current = null
          setLogin(null)
          setOperation(null)
          setNotice(plan ? '' : authorizedNotice ?? (zh ? '授权成功。请点击「管理模型」获取并启用模型，设置会自动保存。' : 'Signed in. Open Models to fetch and enable models. Settings save automatically.'))
          return
        }
        timer = setTimeout(() => void poll(), Math.max(result.interval, 3) * 1000)
      } catch (err) {
        if (stopped) return
        void cancelLogin(login.loginId)
        loginRef.current = null
        setLogin(null)
        setOperation(null)
        setError(String(err))
      }
    }
    timer = setTimeout(() => void poll(), login.interval * 1000)
    return () => { stopped = true; clearTimeout(timer) }
  }, [authorizedNotice, cancelLogin, login, zh])

  const start = async (credentialId?: string) => {
    if (!auth) return
    const run = ++generation.current
    setOperation('sign-in'); setError(''); setNotice('')
    try {
      const next = await api.providerOAuthStart(auth.provider, credentialId)
      if (generation.current !== run) { await cancelLogin(next.loginId); return }
      loginRef.current = next
      setLogin(next)
      if (!next.verificationUrl) return
      try { await api.openExternal(next.verificationUrl) }
      catch { if (generation.current === run) setError(zh ? '无法自动打开浏览器，请点击下方授权链接。' : 'Could not open your browser. Use the authorization link below.') }
    } catch (err) {
      if (generation.current === run) { setError(String(err)); setOperation(null) }
    }
  }

  const cancel = () => {
    generation.current += 1
    if (loginRef.current) void cancelLogin(loginRef.current.loginId)
    loginRef.current = null; setLogin(null); setOperation(null)
  }

  const disconnect = async () => {
    if (!auth?.credentialId) return
    setOperation('disconnect'); setError('')
    try {
      const warning = await api.providerOAuthDisconnect(auth.credentialId)
      const latest = current.current.provider
      if (latest.id !== provider.id || latest.request.oauth?.credentialId !== auth.credentialId) return
      const oauth = chatgpt ? { ...auth, requiresReauthorization: true } : { provider: auth.provider }
      if (chatgpt) setAccountRevision(value => value + 1)
      current.current.onUpdateProvider(latest.id, { request: { ...latest.request, oauth } })
      setNotice(warning ?? '')
    } catch (err) { setError(String(err)) }
    finally { setOperation(null) }
  }

  return <>
    <FieldBlock label={zh ? '认证方式' : 'Authentication'}>
      <Select value={auth?.provider ?? 'api_key'} disabled={busy || Boolean(auth?.credentialId && !auth.requiresReauthorization)}
        onChange={(value) => {
          setError(''); setNotice('')
          if (value === 'api_key') {
            onUpdateProvider(provider.id, { request: { ...provider.request, oauth: null } })
          } else {
            const preset = PROVIDER_PRESETS.find(item => item.oauth === value)
            if (!preset?.oauth) return
            onUpdateProvider(provider.id, {
              request: { ...provider.request, oauth: { provider: preset.oauth } },
              baseUrl: preset.baseUrl,
              apiFormat: preset.apiFormat ?? 'openai_chat',
              availableModels: [], enabledModels: [],
            })
          }
        }} options={[
          { value: 'api_key', label: 'API Key' },
          { value: 'chatgpt', label: 'ChatGPT' },
          { value: 'codex', label: 'Codex OAuth' },
          { value: 'kimi', label: 'Kimi OAuth · Kimi Code' },
          { value: 'antigravity', label: 'Antigravity OAuth · Google' },
        ]} />
    </FieldBlock>
    {auth && <FieldBlock label={zh ? '账号授权' : 'Account authorization'}
      description={chatgpt ? (zh ? '在浏览器中授权 Kivio 使用 ChatGPT 套餐。' : 'Authorize Kivio in your browser to use your ChatGPT plan.') : zh ? '在浏览器中登录账号。凭证保存在本机 JSON 文件中，调用模型时自动刷新；配置备份包含凭证，请勿分享。' : 'Sign in through your browser. Credentials are saved in local JSON and refresh automatically. Configuration backups include credentials; keep them private.'}>
      {chatgpt ? <ChatGPTPlanAccount key={`${auth.credentialId ?? ''}:${Boolean(auth.requiresReauthorization)}:${accountRevision}`}
        provider={provider} settings={settings} lang={lang} busy={busy} accountListRevision={accountListRevision} onSignIn={id => void start(id)}
        onDisconnect={() => void disconnect()} onFetchModels={onFetchModels} onUpdateProvider={onUpdateProvider} /> : <div className="flex items-center gap-2">
        <span className="text-sm" role="status">{auth.credentialId ? (zh ? '已授权' : 'Authorized') : (zh ? '尚未授权' : 'Not signed in')}</span>
        {auth.credentialId
          ? <Button size="sm" disabled={busy} onClick={() => void disconnect()}>{zh ? '断开授权' : 'Disconnect'}</Button>
          : <Button size="sm" disabled={busy} onClick={() => void start()}>{busy ? (zh ? '等待授权…' : 'Waiting…') : (zh ? '登录授权' : 'Sign in')}</Button>}
      </div>}
      {operation === 'sign-in' && <Button size="sm" onClick={cancel}>{zh ? '取消' : 'Cancel'}</Button>}
      {!chatgpt && auth.credentialId && <ProviderAccountIdentity key={`${provider.id}:${auth.provider}:${auth.credentialId}`} provider={provider} lang={lang} />}
      {login && <div className="mt-3 space-y-2 rounded-lg border border-[var(--border)] p-3">
        <p className="text-sm">{login.userCode
          ? (zh ? '在授权页面输入设备码：' : 'Enter this device code on the authorization page:')
          : chatgpt ? (zh ? '请在浏览器中完成 ChatGPT 授权，然后返回 Kivio。' : 'Complete authorization in your browser, then return to Kivio.')
          : (zh ? '请在浏览器中完成 Google 登录，完成后会自动返回授权结果。' : 'Complete Google sign-in in your browser. Kivio will receive the authorization automatically.')}</p>
        {login.userCode && <code className="block select-text text-lg tracking-widest">{login.userCode}</code>}
        {login.verificationUrl && <button type="button" className="kv-provider-link" onClick={() => void api.openExternal(login.verificationUrl).catch(err => setError(String(err)))}>
          {zh ? '打开授权页面 ↗' : 'Open authorization page ↗'}
        </button>}
        <p className="text-xs opacity-70">{zh ? '有效期至 ' : 'Expires at '}{new Date(login.expiresAt * 1000).toLocaleTimeString()}</p>
      </div>}
      {notice && <p className="mt-2 text-sm" role="status">{notice}</p>}
      {error && <ErrorNotice message={error} />}
    </FieldBlock>}
  </>
}
