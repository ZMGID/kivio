import { useEffect, useRef, useState } from 'react'
import { UserRound, UserRoundCheck } from 'lucide-react'
import { api, type ModelProvider, type ProviderOAuthAccount } from '../api/tauri'
import { Button } from '../components/Button'
import type { Lang } from '../components/i18n'

type AccountResult =
  | { status: 'loading' | 'error' }
  | { status: 'ready'; account: ProviderOAuthAccount }

const accountStatus = {
  signedIn: { zh: '已登录', en: 'Signed in', tone: 'ok' },
  signedOut: { zh: '未登录', en: 'Not signed in', tone: 'warn' },
  planRequired: { zh: '待授权套餐', en: 'Grant access', tone: 'warn' },
  reauthorize: { zh: '需重新授权', en: 'Sign in again', tone: 'warn' },
  loading: { zh: '读取账号中', en: 'Checking', tone: 'accent' },
  unavailable: { zh: '账号状态未知', en: 'Status unknown', tone: 'warn' },
}

type AccountHeading = { id: string; name: string }

export function ProviderAccountSummary({ heading, lang, status, identity }: {
  heading: AccountHeading
  lang: Lang
  status: keyof typeof accountStatus
  identity?: string | null
}) {
  const label = accountStatus[status]
  const Icon = status === 'signedIn' ? UserRoundCheck : UserRound
  return <>
    <span className="kv-provider-card-heading">
      <span className="kv-provider-card-name" title={heading.name}>{heading.name}</span>
      <span id={heading.id} className={`kv-tag ${label.tone} kv-provider-account-tag`} title={label[lang]}>
        <Icon size={12} aria-hidden="true" />
        <span>{label[lang]}</span>
      </span>
    </span>
    {identity && <span id={`${heading.id}-identity`} className="kv-provider-card-url" title={identity}>{identity}</span>}
  </>
}

export function ProviderAccountIdentity({ provider, lang, heading }: {
  provider: ModelProvider
  lang: Lang
  heading?: AccountHeading
}) {
  const [result, setResult] = useState<AccountResult>({ status: 'loading' })
  const [retry, setRetry] = useState(0)
  const current = useRef(provider)
  current.current = provider
  const zh = lang === 'zh'
  useEffect(() => {
    let stopped = false
    setResult({ status: 'loading' })
    void (async () => {
      try {
        const account = await api.providerOAuthAccount(current.current)
        if (!stopped) setResult({ status: 'ready', account })
      } catch { if (!stopped) setResult({ status: 'error' }) }
    })()
    return () => { stopped = true }
  }, [provider.id, provider.request.oauth?.provider, provider.request.oauth?.credentialId, provider.baseUrl, provider.apiFormat, provider.request.useSystemProxy, retry])
  const { status } = result
  const account = result.status === 'ready' ? result.account : null
  if (heading) return <ProviderAccountSummary heading={heading} lang={lang}
    status={status === 'ready' ? 'signedIn' : status === 'error' ? 'unavailable' : 'loading'}
    identity={account?.email || account?.name || account?.accountId} />
  return <div className="mt-2 rounded-lg border border-[var(--border)] px-3 py-2 text-sm" aria-label={zh ? '当前授权账号' : 'Authorized account'}>
    <div className="mb-1 text-xs opacity-60">{zh ? '当前账号' : 'Current account'}</div>
    {status === 'loading' && <span role="status" className="text-xs opacity-60">{zh ? '正在读取账号…' : 'Loading account…'}</span>}
    {status === 'error' && <div className="flex items-center justify-between gap-2">
      <span className="text-xs opacity-70">{zh ? '账号信息暂时无法读取' : 'Account information unavailable'}</span>
      <Button size="sm" onClick={() => setRetry(n => n + 1)}>{zh ? '重试' : 'Retry'}</Button>
    </div>}
    {status === 'ready' && <>
      {account?.email && <div className="break-all select-text font-medium">{account.email}</div>}
      {account?.name && account.name !== account.email && <div className="break-words">{account.name}</div>}
      {account?.accountId && <div className="break-all select-text text-xs opacity-70">{zh ? '账号 ID：' : 'Account ID: '}{account.accountId}</div>}
      {!account?.email && !account?.name && !account?.accountId && <span className="text-xs opacity-60">{zh ? '供应商未返回账号标识' : 'The provider did not return an account identity'}</span>}
    </>}
  </div>
}
