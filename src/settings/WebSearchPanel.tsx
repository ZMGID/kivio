import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { ImagePlus, X, AlertCircle, CheckCircle2, Eye, EyeOff, ExternalLink, Globe, Loader2, Search, SlidersHorizontal } from 'lucide-react'
import { api, type Settings, type WebSearchMcpAuth, type WebSearchProviderId } from '../api/tauri'
import type { I18n } from '../components/i18n'
import { Input, Select, SettingRow, Toggle } from './components'
import { PromptField } from './ScreenshotTranslationSettings'
import { Button, IconButton } from '../components/Button'
import { ErrorNotice } from '../components/AppNotice'
import { Row, Section } from './externalAgents/shared'
import { isProviderConfigured, providerSupportsFetch, resolvedFetchProvider } from './webSearch'

type WebSearchConfig = NonNullable<Settings['lens']['webSearch']>
/** 后端已接入的搜索源（settings 的 provider 枚举值）。 */
type ProviderId = WebSearchProviderId

type WebSearchPanelProps = {
  t: I18n
  webSearch: WebSearchConfig | undefined
  onChange: (updates: Partial<WebSearchConfig>) => void
}

const DEFAULT_WEB_SEARCH: WebSearchConfig = {
  enabled: false,
  provider: 'tavily',
  tavilyApiKey: '',
  exaApiKey: '',
  exaMcpUrl: 'https://mcp.exa.ai/mcp',
  ollamaApiKey: '',
  grokApiKey: '',
  grokModel: 'grok-4-1-fast-non-reasoning',
  grokBaseUrl: 'https://api.x.ai/v1',
  grokSystemPrompt:
    "You are a helpful search assistant. Search the web to find accurate and up-to-date information for the user's query. Provide a comprehensive answer with citations.",
  deepseekApiKey: '',
  deepseekBaseUrl: 'https://api.deepseek.com',
  braveApiKey: '',
  braveBaseUrl: 'https://api.search.brave.com',
  serperApiKey: '',
  serperBaseUrl: 'https://google.serper.dev',
  bochaApiKey: '',
  bochaBaseUrl: 'https://api.bochaai.com',
  zhipuApiKey: '',
  zhipuBaseUrl: 'https://open.bigmodel.cn/api/paas/v4',
  tinyfishApiKey: '',
  tinyfishBaseUrl: 'https://api.search.tinyfish.ai',
  tinyfishMcpUrl: 'https://agent.tinyfish.ai/mcp',
  tinyfishMcpAuth: null,
  searxngBaseUrl: '',
  kimiApiKey: '',
  kimiBaseUrl: 'https://api.kimi.com/coding/v1/search',
  keenableApiKey: '',
  keenableBaseUrl: 'https://api.keenable.ai',
  specstreamApiKey: '',
  specstreamBaseUrl: 'https://api.specstream.ai',
  maxResults: 5,
  searchDepth: 'basic',
}

type ProviderDef = {
  id: ProviderId
  name: string
  site: string
  apiKeyUrl?: string
  /** 使用哪个密钥字段；ExaMCP / SearXNG 无需密钥则留空。 */
  keyField?:
    | 'tavilyApiKey'
    | 'exaApiKey'
    | 'ollamaApiKey'
    | 'grokApiKey'
    | 'deepseekApiKey'
    | 'braveApiKey'
    | 'serperApiKey'
    | 'bochaApiKey'
    | 'zhipuApiKey'
    | 'tinyfishApiKey'
    | 'kimiApiKey'
    | 'keenableApiKey'
    | 'specstreamApiKey'
  /** API Key 输入框占位符。 */
  keyPlaceholder?: string
  /** 可编辑的 API base 字段（代理/自建网关可改）；写入对应 *BaseUrl 设置。 */
  baseUrlField?:
    | 'tavilyBaseUrl'
    | 'exaBaseUrl'
    | 'ollamaBaseUrl'
    | 'braveBaseUrl'
    | 'serperBaseUrl'
    | 'bochaBaseUrl'
    | 'zhipuBaseUrl'
    | 'tinyfishBaseUrl'
    | 'searxngBaseUrl'
    | 'grokBaseUrl'
    | 'deepseekBaseUrl'
    | 'kimiBaseUrl'
    | 'keenableBaseUrl'
    | 'specstreamBaseUrl'
  /** base 输入框占位符（也是官方默认值）。 */
  baseUrlPlaceholder?: string
  /** 可编辑 MCP endpoint（写入 exaMcpUrl / tinyfishMcpUrl）。 */
  endpointField?: 'exaMcpUrl' | 'tinyfishMcpUrl'
  endpointPlaceholder?: string
  /** 地址栏下方说明：keyless MCP / 自建实例。 */
  urlHint?: 'exaMcp' | 'tinyfishMcp' | 'searxng' | 'kimi' | 'deepseek'
  supportsDepth?: boolean
  /** 模型驱动搜索（Grok）：额外显示 型号 / 自定义网址 / 系统提示。 */
  modelBased?: boolean
  modelField?: 'grokModel'
  modelPlaceholder?: string
  systemPromptField?: 'grokSystemPrompt'
  /** 本地品牌图标路径（public/），无则回退到文字标。 */
  icon?: string
}

const PROVIDERS: ProviderDef[] = [
  {
    id: 'bocha',
    name: 'Bocha',
    site: 'https://open.bochaai.com',
    apiKeyUrl: 'https://open.bochaai.com',
    keyField: 'bochaApiKey',
    keyPlaceholder: 'bocha key',
    baseUrlField: 'bochaBaseUrl',
    baseUrlPlaceholder: 'https://api.bochaai.com',
    icon: '/search-icons/bocha.png',
  },
  {
    id: 'brave',
    name: 'Brave',
    site: 'https://brave.com/search/api/',
    apiKeyUrl: 'https://api-dashboard.search.brave.com/app/keys',
    keyField: 'braveApiKey',
    keyPlaceholder: 'BSA...',
    baseUrlField: 'braveBaseUrl',
    baseUrlPlaceholder: 'https://api.search.brave.com',
    icon: '/search-icons/brave.svg',
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    site: 'https://api-docs.deepseek.com/guides/search/',
    apiKeyUrl: 'https://platform.deepseek.com/api_keys',
    keyField: 'deepseekApiKey',
    keyPlaceholder: 'sk-...',
    baseUrlField: 'deepseekBaseUrl',
    baseUrlPlaceholder: 'https://api.deepseek.com',
    urlHint: 'deepseek',
    icon: '/search-icons/deepseek.svg',
  },
  {
    id: 'exa',
    name: 'Exa',
    site: 'https://exa.ai',
    apiKeyUrl: 'https://dashboard.exa.ai/api-keys',
    keyField: 'exaApiKey',
    keyPlaceholder: 'exa-...',
    baseUrlField: 'exaBaseUrl',
    baseUrlPlaceholder: 'https://api.exa.ai',
    icon: '/search-icons/exa.png',
  },
  {
    id: 'exa_mcp',
    name: 'ExaMCP',
    site: 'https://docs.exa.ai/reference/exa-mcp',
    // Exa MCP 无需 API Key，只需 endpoint。
    endpointField: 'exaMcpUrl',
    endpointPlaceholder: 'https://mcp.exa.ai/mcp',
    urlHint: 'exaMcp',
    icon: '/search-icons/exa.png',
  },
  {
    id: 'grok',
    name: 'Grok',
    site: 'https://x.ai',
    apiKeyUrl: 'https://console.x.ai',
    keyField: 'grokApiKey',
    keyPlaceholder: 'xai-...',
    modelBased: true,
    modelField: 'grokModel',
    modelPlaceholder: 'grok-4-1-fast-non-reasoning',
    baseUrlField: 'grokBaseUrl',
    baseUrlPlaceholder: 'https://api.x.ai/v1',
    systemPromptField: 'grokSystemPrompt',
    icon: '/search-icons/grok.png',
  },
  {
    id: 'keenable',
    name: 'Keenable',
    site: 'https://keenable.ai',
    apiKeyUrl: 'https://keenable.ai',
    // Works without a key (public endpoint); a key is optional.
    keyField: 'keenableApiKey',
    keyPlaceholder: 'optional',
    baseUrlField: 'keenableBaseUrl',
    baseUrlPlaceholder: 'https://api.keenable.ai',
  },
  {
    id: 'kimi',
    name: 'Kimi',
    site: 'https://www.kimi.com/code',
    apiKeyUrl: 'https://www.kimi.com/code/console',
    keyField: 'kimiApiKey',
    keyPlaceholder: 'sk-...',
    baseUrlField: 'kimiBaseUrl',
    baseUrlPlaceholder: 'https://api.kimi.com/coding/v1/search',
    urlHint: 'kimi',
    icon: '/search-icons/kimi.svg',
  },
  {
    id: 'ollama',
    name: 'Ollama',
    site: 'https://ollama.com',
    apiKeyUrl: 'https://ollama.com/settings/keys',
    keyField: 'ollamaApiKey',
    keyPlaceholder: 'ollama key',
    baseUrlField: 'ollamaBaseUrl',
    baseUrlPlaceholder: 'https://ollama.com',
    icon: '/search-icons/ollama.png',
  },
  {
    id: 'searxng',
    name: 'SearXNG',
    site: 'https://docs.searxng.org/dev/search_api.html',
    baseUrlField: 'searxngBaseUrl',
    baseUrlPlaceholder: 'https://searx.example',
    urlHint: 'searxng',
    icon: '/search-icons/searxng.svg',
  },
  {
    id: 'serper',
    name: 'Serper',
    site: 'https://serper.dev',
    apiKeyUrl: 'https://serper.dev/api-key',
    keyField: 'serperApiKey',
    keyPlaceholder: 'serper key',
    baseUrlField: 'serperBaseUrl',
    baseUrlPlaceholder: 'https://google.serper.dev',
    icon: '/search-icons/serper.png',
  },
  {
    id: 'specstream',
    name: 'SpecStream',
    site: 'https://specstream.ai/search-api',
    apiKeyUrl: 'https://specstream.ai/signup?src=kivio',
    keyField: 'specstreamApiKey',
    keyPlaceholder: 'specstream key',
    baseUrlField: 'specstreamBaseUrl',
    baseUrlPlaceholder: 'https://api.specstream.ai',
    icon: '/search-icons/specstream.svg',
  },
  {
    id: 'tavily',
    name: 'Tavily',
    site: 'https://tavily.com',
    apiKeyUrl: 'https://app.tavily.com/home',
    keyField: 'tavilyApiKey',
    keyPlaceholder: 'tvly-...',
    baseUrlField: 'tavilyBaseUrl',
    baseUrlPlaceholder: 'https://api.tavily.com',
    supportsDepth: true,
    icon: '/search-icons/tavily.png',
  },
  {
    id: 'tinyfish',
    name: 'TinyFish',
    site: 'https://www.tinyfish.ai',
    apiKeyUrl: 'https://agent.tinyfish.ai/api-keys',
    keyField: 'tinyfishApiKey',
    keyPlaceholder: 'tinyfish key',
    baseUrlField: 'tinyfishBaseUrl',
    baseUrlPlaceholder: 'https://api.search.tinyfish.ai',
    icon: '/search-icons/tinyfish.png',
  },
  {
    id: 'tinyfish_mcp',
    name: 'TinyFish MCP',
    site: 'https://docs.tinyfish.ai/mcp-integration',
    endpointField: 'tinyfishMcpUrl',
    endpointPlaceholder: 'https://agent.tinyfish.ai/mcp',
    urlHint: 'tinyfishMcp',
    icon: '/search-icons/tinyfish.png',
  },
  {
    id: 'zhipu',
    name: 'Zhipu',
    site: 'https://open.bigmodel.cn',
    apiKeyUrl: 'https://open.bigmodel.cn/usercenter/apikeys',
    keyField: 'zhipuApiKey',
    keyPlaceholder: 'zhipu key',
    baseUrlField: 'zhipuBaseUrl',
    baseUrlPlaceholder: 'https://open.bigmodel.cn/api/paas/v4',
    icon: '/search-icons/zhipu.png',
  },
]

/** 品牌图标：优先本地真实 logo（public/search-icons），加载失败回退到首字母。 */
function ProviderMark({ name, icon, size = 20 }: { name: string; icon?: string; size?: number }) {
  const [failed, setFailed] = useState(false)
  if (icon && !failed) {
    return (
      <img
        src={icon}
        alt=""
        width={size}
        height={size}
        draggable={false}
        onError={() => setFailed(true)}
        className="shrink-0 rounded-[4px] object-contain"
        style={{ width: size, height: size }}
      />
    )
  }
  return (
    <span
      className="grid shrink-0 place-items-center font-semibold uppercase text-[var(--text-muted)]"
      style={{ width: size, height: size, fontSize: size * 0.62, lineHeight: 1 }}
    >
      {name.slice(0, 1)}
    </span>
  )
}

/** 与本地 CLI 页同款的描边图标框，服务商 logo 或线性图标放在里面。 */
function IconFrame({ size, children }: { size: number; children: ReactNode }) {
  return (
    <span
      className="kv-cli-iconbox text-[var(--text-muted)]"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      {children}
    </span>
  )
}

function ProviderFrame({ provider, size }: { provider: ProviderDef; size: number }) {
  return (
    <IconFrame size={size}>
      <ProviderMark name={provider.name} icon={provider.icon} size={Math.round(size * 0.6)} />
    </IconFrame>
  )
}

function hostname(url: string): string {
  return url.replace(/^https?:\/\//, '').replace(/\/.*$/, '')
}

function ApiKeyField({
  t,
  value,
  placeholder,
  onChange,
}: {
  t: I18n
  value: string
  placeholder: string
  onChange: (value: string) => void
}) {
  const [reveal, setReveal] = useState(false)
  const label = reveal ? t.externalAgentsProviderHideKey : t.externalAgentsProviderShowKey
  return (
    <div className="flex w-full items-center gap-1.5">
      <Input
        type={reveal ? 'text' : 'password'}
        value={value}
        onChange={onChange}
        placeholder={placeholder}
        mono
        autoComplete="off"
        spellCheck={false}
        className="min-w-0 flex-1"
      />
      <IconButton
        variant="ghost"
        label={label}
        onClick={() => setReveal((v) => !v)}
        data-tauri-drag-region="false"
      >
        {reveal ? <EyeOff size={14} /> : <Eye size={14} />}
      </IconButton>
    </div>
  )
}

function TinyfishMcpAuthRow({
  t,
  url,
  auth,
  onChange,
}: {
  t: I18n
  url: string
  auth: WebSearchMcpAuth | null | undefined
  onChange: (auth: WebSearchMcpAuth | null) => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)
  const attempt = useRef<AbortController | null>(null)
  const authorized = (auth?.accessToken ?? '').trim() !== ''
  const account = auth?.account?.trim()

  // Leaving this row cancels the window's connector OAuth flow. connectorOauthConnect
  // aborts via connector_oauth_cancel; a late server must not be written back.
  useEffect(() => () => {
    generation.current += 1
    attempt.current?.abort()
    attempt.current = null
  }, [])

  const authorize = useCallback(async () => {
    if (attempt.current) return
    const endpoint = url.trim() || 'https://agent.tinyfish.ai/mcp'
    const run = generation.current + 1
    generation.current = run
    const controller = new AbortController()
    attempt.current = controller
    setError(null)
    setBusy(true)
    try {
      const server = await api.connectorOauthConnect({
        url: endpoint,
        name: 'TinyFish MCP',
      }, undefined, controller.signal)
      if (generation.current !== run || controller.signal.aborted) return
      const next = server.auth?.accessToken?.trim()
        ? server.auth
        : authFromOauthHeader(server.headers)
      if (generation.current !== run || controller.signal.aborted) return
      if (!next) {
        setError(t.webSearchTinyfishMcpAuthFailed)
        return
      }
      onChange(next)
    } catch (err) {
      if (generation.current !== run || controller.signal.aborted) return
      const message = err instanceof Error ? err.message : String(err)
      if (message.includes('OAUTH_CANCELLED')) return
      setError(message)
    } finally {
      if (attempt.current === controller) attempt.current = null
      if (generation.current === run) setBusy(false)
    }
  }, [onChange, t.webSearchTinyfishMcpAuthFailed, url])

  return (
    <SettingRow
      label={t.connectorsAuthOauth}
      description={
        authorized
          ? (account
            ? t.webSearchTinyfishMcpAuthorizedAs.replace('{account}', account)
            : t.webSearchTinyfishMcpAuthorized)
          : t.webSearchTinyfishMcpHint
      }
    >
      <div className="flex flex-col items-end gap-1">
        <div className="flex items-center gap-2">
          {authorized ? (
            <Button
              onClick={() => {
                setError(null)
                onChange(null)
              }}
              data-tauri-drag-region="false"
            >
              {t.webSearchTinyfishMcpDisconnect}
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void authorize()}
              data-tauri-drag-region="false"
            >
              {busy ? t.webSearchTinyfishMcpAuthorizing : t.webSearchTinyfishMcpAuthorize}
            </Button>
          )}
        </div>
        <ErrorNotice message={error} />
      </div>
    </SettingRow>
  )
}

function authFromOauthHeader(headers: Record<string, string>): WebSearchMcpAuth | null {
  const header = Object.entries(headers).find(([key]) => key.toLowerCase() === 'authorization')?.[1] ?? ''
  const token = header.replace(/^Bearer\s+/i, '').trim()
  if (!token) return null
  return { kind: 'oauth', accessToken: token }
}

type TestState =
  | { status: 'idle' }
  | { status: 'running' }
  | { status: 'ok'; count: number; results: { title: string; url: string; imageUrl?: string }[] }
  | { status: 'error'; message: string }

/** 测试搜索：用当前（可能未保存的）配置真实跑一次搜索，展示结果或错误。 */
function TestSearch({ t, config, desc }: { t: I18n; config: WebSearchConfig; desc?: string }) {
  const [query, setQuery] = useState('')
  const [imagePath, setImagePath] = useState<string | null>(null)
  const [state, setState] = useState<TestState>({ status: 'idle' })
  const running = state.status === 'running'

  const run = async () => {
    const q = query.trim()
    if ((!q && !imagePath) || running) return
    setState({ status: 'running' })
    try {
      const res = await api.testWebSearch(config, q, imagePath ?? undefined)
      if (res.success) {
        const results = res.results ?? []
        setState({ status: 'ok', count: results.length, results: results.slice(0, 5) })
      } else {
        setState({ status: 'error', message: res.error || 'Unknown error' })
      }
    } catch (err) {
      setState({ status: 'error', message: err instanceof Error ? err.message : String(err) })
    }
  }

  const chooseImage = async () => {
    try {
      const selected = await open({ multiple: false, filters: [{ name: t.webSearchChooseImage, extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif'] }] })
      if (selected) {
        setImagePath(selected)
        setState({ status: 'idle' })
      }
    } catch (err) {
      setState({ status: 'error', message: String(err) })
    }
  }

  return (
    <Section title={t.webSearchTestSection} desc={desc}>
      <div className="kv-cli-card">
        {config.provider === 'deepseek' && (
          <div className="flex min-w-0 items-center gap-2 pt-3">
            <Button size="sm" onClick={() => void chooseImage()} disabled={running}>
              <ImagePlus size={14} />{t.webSearchChooseImage}
            </Button>
            {imagePath && <>
              <span className="min-w-0 truncate text-xs">{imagePath.split(/[\\/]/).pop()}</span>
              <IconButton size="xs" label={t.webSearchRemoveImage} disabled={running} onClick={() => { setImagePath(null); setState({ status: 'idle' }) }}>
                <X size={13} />
              </IconButton>
            </>}
          </div>
        )}
        <div className="flex items-center gap-2 py-3">
          <Input
            value={query}
            onChange={setQuery}
            placeholder={imagePath ? t.webSearchImageQuery : t.webSearchTestPlaceholder}
            aria-label={imagePath ? t.webSearchImageQuery : t.webSearchTestPlaceholder}
            className="min-w-0 flex-1"
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing || event.key !== 'Enter') return
              event.preventDefault()
              void run()
            }}
          />
          <Button
            onClick={() => void run()}
            disabled={(!query.trim() && !imagePath) || running}
            data-tauri-drag-region="false"
          >
            {running ? <Loader2 size={13} className="animate-spin" /> : <Search size={13} />}
            {running ? t.webSearchTesting : t.webSearchTestRun}
          </Button>
        </div>

        {state.status === 'error' && (
          <p className="flex items-start gap-1.5 border-t border-[var(--border)] py-3 text-[12px] text-danger">
            <AlertCircle size={13} className="mt-px shrink-0" />
            <span className="min-w-0 break-all">{state.message}</span>
          </p>
        )}
        {state.status === 'ok' && (
          <div className="border-t border-[var(--border)] py-3">
            <p className="flex items-center gap-1.5 text-[12px] text-success">
              <CheckCircle2 size={13} className="shrink-0" />
              {state.count > 0
                ? t.webSearchTestResults.replace('{n}', String(state.count))
                : t.webSearchTestEmpty}
            </p>
            {state.results.length > 0 && (
              <ol className="mt-2 space-y-1">
                {state.results.map((r, i) => (
                  <li key={`${r.url}-${i}`} className="flex min-w-0 items-baseline gap-2 text-[12px]">
                    <span className="w-3 shrink-0 text-right tabular-nums text-[var(--text-faint)]">{i + 1}</span>
                    {imagePath ? (
                      <a href={r.url} className="min-w-0 flex-1 truncate" onClick={(event) => {
                        event.preventDefault()
                        void api.openExternal(r.url)
                      }}>{r.title || r.url}</a>
                    ) : <span className="min-w-0 flex-1 truncate text-[var(--text)]">{r.title || r.url}</span>}
                    {r.imageUrl && (
                      <a href={r.imageUrl} onClick={(event) => {
                        event.preventDefault()
                        void api.openExternal(event.currentTarget.href)
                      }}>{t.webSearchOpenImage}</a>
                    )}
                    <span className="max-w-[40%] shrink-0 truncate font-mono text-[11px] text-[var(--text-faint)]">
                      {hostname(r.url)}
                    </span>
                  </li>
                ))}
              </ol>
            )}
          </div>
        )}
      </div>
    </Section>
  )
}

const FETCH_FOLLOW = 'follow'

/** 换搜索源。它原本被单独选作 Fetch 时改回「跟随搜索」，同一家不在两个字段里各存一份。 */
function searchProviderPatch(config: WebSearchConfig, id: ProviderId): Partial<WebSearchConfig> {
  return { provider: id, ...(config.fetchProvider === id ? { fetchProvider: null } : {}) }
}

/** 换 Fetch 源。选「跟随」或与搜索同一家时清空，交给搜索源决定。 */
function fetchProviderPatch(config: WebSearchConfig, value: string): Partial<WebSearchConfig> {
  return value === FETCH_FOLLOW || value === config.provider
    ? { fetchProvider: null }
    : { fetchProvider: value as ProviderId }
}

function findProvider(id: ProviderId): ProviderDef | undefined {
  return PROVIDERS.find((provider) => provider.id === id)
}

/** 默认页里服务商没配好时的提示：一句状态加一个跳到该服务商的链接。 */
function SetupHint({ t, onConfigure }: { t: I18n; onConfigure: () => void }) {
  return (
    <span className="inline-flex flex-wrap items-center gap-x-1.5 text-[var(--warn)]">
      {t.webSearchStatusMissing}
      <button
        type="button"
        onClick={onConfigure}
        className="text-accent hover:underline"
        data-tauri-drag-region="false"
      >
        {t.webSearchConfigureProvider} →
      </button>
    </span>
  )
}

function DefaultsPage({
  t,
  config,
  onChange,
  onOpenProvider,
}: {
  t: I18n
  config: WebSearchConfig
  onChange: (updates: Partial<WebSearchConfig>) => void
  onOpenProvider: (id: ProviderId) => void
}) {
  const search = findProvider(config.provider)
  const searchName = search?.name ?? config.provider
  const followsSearch = !config.fetchProvider || config.fetchProvider === config.provider
  const resolvedFetch = resolvedFetchProvider(config)
  const fetch = resolvedFetch ? findProvider(resolvedFetch) : undefined
  const searchReady = isProviderConfigured(config, config.provider)
  const fetchReady = !resolvedFetch || isProviderConfigured(config, resolvedFetch)
  const fetchOptions = [
    {
      value: FETCH_FOLLOW,
      label: providerSupportsFetch(config.provider)
        ? t.webSearchFetchFollow.replace('{name}', searchName)
        : t.webSearchFetchFollowDirect,
    },
    ...PROVIDERS
      .filter((provider) => providerSupportsFetch(provider.id) && provider.id !== config.provider)
      .map((provider) => ({ value: provider.id, label: provider.name })),
  ]

  return (
    <>
      <header className="kv-cli-head">
        <IconFrame size={40}>
          <SlidersHorizontal size={18} />
        </IconFrame>
        <div className="kv-cli-head-main">
          <h2 className="kv-cli-head-title">{t.webSearchDefaultsNav}</h2>
        </div>
      </header>

      <Section title={t.webSearchDefaultsTitle} desc={t.webSearchRolesHint}>
        <div className="kv-cli-card">
          <Row
            lead={search ? <ProviderFrame provider={search} size={28} /> : undefined}
            label={t.webSearchSearchProvider}
            desc={searchReady
              ? undefined
              : <SetupHint t={t} onConfigure={() => onOpenProvider(config.provider)} />}
          >
            <Select
              className="kv-ctl"
              value={config.provider}
              onChange={(value) => onChange(searchProviderPatch(config, value as ProviderId))}
              options={PROVIDERS.map((provider) => ({ value: provider.id, label: provider.name }))}
            />
          </Row>
          <Row
            lead={fetch
              ? <ProviderFrame provider={fetch} size={28} />
              : <IconFrame size={28}><Globe size={14} /></IconFrame>}
            label={t.webSearchFetchProvider}
            desc={!resolvedFetch
              ? t.webSearchFetchDirectHint
              : fetchReady
                ? undefined
                : <SetupHint t={t} onConfigure={() => onOpenProvider(resolvedFetch)} />}
          >
            <Select
              className="kv-ctl"
              value={followsSearch ? FETCH_FOLLOW : config.fetchProvider!}
              onChange={(value) => onChange(fetchProviderPatch(config, value))}
              options={fetchOptions}
            />
          </Row>
        </div>
      </Section>

      <Section title={t.webSearchGeneralSection}>
        <div className="kv-cli-card">
          <Row label={t.lensWebSearchMaxResults} desc={t.webSearchMaxResultsHint}>
            <Input
              type="number"
              min={1}
              max={10}
              className="kv-ctl-num"
              aria-label={t.lensWebSearchMaxResults}
              value={String(config.maxResults ?? 5)}
              onChange={(value) => onChange({
                maxResults: Math.min(10, Math.max(1, Number.parseInt(value, 10) || 1)),
              })}
            />
          </Row>
        </div>
      </Section>

      <Section title={t.webSearchUsageSection}>
        <div className="kv-cli-card">
          <Row label={t.webSearchLensSection} desc={t.lensWebSearchHint}>
            <Toggle
              checked={config.enabled === true}
              onChange={(enabled) => onChange({ enabled })}
              ariaLabel={t.webSearchLensSection}
            />
          </Row>
          <Row label={t.webSearchChatSection} desc={t.webSearchChatToolsHint} />
        </div>
      </Section>

      <TestSearch key={config.provider} t={t} config={config} desc={t.webSearchTestDefaultsHint} />
    </>
  )
}

function ProviderDetail({
  t,
  provider,
  config,
  onChange,
}: {
  t: I18n
  provider: ProviderDef
  config: WebSearchConfig
  onChange: (updates: Partial<WebSearchConfig>) => void
}) {
  const keyValue = provider.keyField ? config[provider.keyField] || '' : ''
  const ready = isProviderConfigured(config, provider.id)
  const supportsFetch = providerSupportsFetch(provider.id)
  const usedForSearch = config.provider === provider.id
  const usedForFetch = resolvedFetchProvider(config) === provider.id
  const urlHints: Record<NonNullable<ProviderDef['urlHint']>, string> = {
    searxng: t.webSearchSearxngHint,
    exaMcp: t.webSearchExaMcpKeyless,
    tinyfishMcp: t.webSearchTinyfishMcpHint,
    kimi: t.webSearchKimiHint,
    deepseek: t.webSearchDeepseekHint,
  }
  const urlHint = provider.urlHint ? urlHints[provider.urlHint] : undefined
  const inUse = <span className="kv-tag accent">{t.webSearchInUse}</span>

  return (
    <>
      <header className="kv-cli-head">
        <ProviderFrame provider={provider} size={40} />
        <div className="kv-cli-head-main">
          <h2 className="kv-cli-head-title">{provider.name}</h2>
          <span className="kv-cli-version min-w-0 truncate">{hostname(provider.site)}</span>
          <span className={`kv-tag ${ready ? 'ok' : 'warn'}`}>
            {ready ? t.webSearchStatusReady : t.webSearchStatusMissing}
          </span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void api.openExternal(provider.site)}
          data-tauri-drag-region="false"
        >
          <ExternalLink size={12} />
          {t.webSearchOpenSite}
        </Button>
      </header>

      <Section title={t.webSearchRolesSection}>
        <div className="kv-cli-card">
          <Row label={t.webSearchSearchProvider}>
            {usedForSearch ? inUse : (
              <Button
                size="sm"
                onClick={() => onChange(searchProviderPatch(config, provider.id))}
                data-tauri-drag-region="false"
              >
                {t.webSearchSetDefault}
              </Button>
            )}
          </Row>
          <Row
            label={t.webSearchFetchProvider}
            desc={supportsFetch ? undefined : t.webSearchFetchUnsupported}
          >
            {supportsFetch && (usedForFetch ? inUse : (
              <Button
                size="sm"
                onClick={() => onChange(fetchProviderPatch(config, provider.id))}
                data-tauri-drag-region="false"
              >
                {t.webSearchSetDefault}
              </Button>
            ))}
          </Row>
        </div>
      </Section>

      <Section
        title={t.webSearchProviderSection}
        desc={provider.keyField && !ready ? t.webSearchApiKeyRequired : undefined}
        actions={provider.apiKeyUrl && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void api.openExternal(provider.apiKeyUrl!)}
            data-tauri-drag-region="false"
          >
            {t.webSearchGetKey}
            <ExternalLink size={12} />
          </Button>
        )}
      >
        <div className="kv-cli-card">
          {provider.keyField && (
            <SettingRow label={t.lensWebSearchApiKey} stack>
              <ApiKeyField
                t={t}
                value={keyValue}
                placeholder={provider.keyPlaceholder || 'API key'}
                onChange={(value) => onChange({ [provider.keyField!]: value } as Partial<WebSearchConfig>)}
              />
            </SettingRow>
          )}

          {!provider.modelBased && (
            <SettingRow label={t.webSearchApiUrl} description={urlHint} stack>
              {provider.endpointField ? (
                <Input
                  value={config[provider.endpointField] || ''}
                  onChange={(value) => onChange({ [provider.endpointField!]: value } as Partial<WebSearchConfig>)}
                  placeholder={provider.endpointPlaceholder}
                  mono
                  className="w-full"
                />
              ) : provider.baseUrlField ? (
                <Input
                  value={config[provider.baseUrlField] ?? ''}
                  onChange={(value) => onChange({ [provider.baseUrlField!]: value } as Partial<WebSearchConfig>)}
                  placeholder={provider.baseUrlPlaceholder}
                  mono
                  className="w-full"
                />
              ) : null}
            </SettingRow>
          )}

          {provider.urlHint === 'tinyfishMcp' && (
            <TinyfishMcpAuthRow
              t={t}
              url={config.tinyfishMcpUrl || ''}
              auth={config.tinyfishMcpAuth}
              onChange={(tinyfishMcpAuth) => onChange({ tinyfishMcpAuth })}
            />
          )}

          {provider.modelBased && provider.modelField && provider.systemPromptField && (
            <>
              <SettingRow label={t.webSearchModel} stack>
                <Input
                  value={String(config[provider.modelField] || '')}
                  onChange={(value) => onChange({ [provider.modelField!]: value } as Partial<WebSearchConfig>)}
                  placeholder={provider.modelPlaceholder}
                  mono
                  className="w-full"
                />
              </SettingRow>
              <SettingRow label={t.webSearchCustomUrl} stack>
                <Input
                  value={provider.baseUrlField ? String(config[provider.baseUrlField] ?? '') : ''}
                  onChange={(value) => provider.baseUrlField
                    && onChange({ [provider.baseUrlField]: value } as Partial<WebSearchConfig>)}
                  placeholder={provider.baseUrlPlaceholder}
                  mono
                  className="w-full"
                />
              </SettingRow>
              <PromptField
                label={t.webSearchSystemPrompt}
                value={String(config[provider.systemPromptField] || '')}
                defaultText=""
                t={t}
                onChange={(value) => onChange({ [provider.systemPromptField!]: value } as Partial<WebSearchConfig>)}
              />
            </>
          )}

          {provider.supportsDepth && (
            <SettingRow label={t.lensWebSearchDepth}>
              <Select
                className="w-44"
                value={config.searchDepth || 'basic'}
                onChange={(v) => onChange({ searchDepth: v as WebSearchConfig['searchDepth'] })}
                options={[
                  { value: 'ultra-fast', label: 'Ultra fast' },
                  { value: 'fast', label: 'Fast' },
                  { value: 'basic', label: 'Basic' },
                  { value: 'advanced', label: 'Advanced' },
                ]}
              />
            </SettingRow>
          )}
        </div>
      </Section>

      <TestSearch key={provider.id} t={t} config={{ ...config, provider: provider.id }} />
    </>
  )
}

/** 左栏一行：图标、名称，右侧是它当前承担的角色和是否已配置。 */
function NavItem({
  active,
  label,
  icon,
  trailing,
  describedBy,
  onSelect,
}: {
  active: boolean
  label: string
  icon: ReactNode
  trailing?: ReactNode
  describedBy?: string
  onSelect: () => void
}) {
  return (
    <div className={`kv-cli-nav-item ${active ? 'active' : ''}`}>
      <button
        type="button"
        className="kv-cli-nav-select"
        onClick={onSelect}
        aria-label={label}
        aria-describedby={describedBy}
        aria-current={active ? 'true' : undefined}
        data-tauri-drag-region="false"
      >
        {icon}
        <span className="kv-cli-nav-text">
          <span className="kv-cli-nav-name">{label}</span>
        </span>
        {trailing}
      </button>
    </div>
  )
}

export function WebSearchPanel({ t, webSearch, onChange }: WebSearchPanelProps) {
  const config = { ...DEFAULT_WEB_SEARCH, ...(webSearch ?? {}) }
  const [selectedId, setSelectedId] = useState<ProviderId | null>(null)
  const statusId = useId()
  const selected = selectedId ? findProvider(selectedId) : undefined
  const resolvedFetch = resolvedFetchProvider(config)

  return (
    <div className="kv-providers-root">
      <div className="kv-cli">
        <nav className="kv-cli-nav kv-websearch-nav" aria-label={t.tabWebSearch}>
          <div className="kv-cli-nav-list">
            <NavItem
              active={!selected}
              label={t.webSearchDefaultsNav}
              icon={<IconFrame size={24}><SlidersHorizontal size={13} /></IconFrame>}
              onSelect={() => setSelectedId(null)}
            />
          </div>
          <div className="kv-websearch-nav-group">{t.webSearchApiSection}</div>
          <div className="kv-cli-nav-list">
            {PROVIDERS.map((provider) => {
              const ready = isProviderConfigured(config, provider.id)
              const roles: string[] = []
              if (config.provider === provider.id) roles.push(t.webSearchSearchBadge)
              if (resolvedFetch === provider.id) roles.push(t.webSearchFetchBadge)
              const status = ready ? t.webSearchStatusReady : t.webSearchStatusMissing
              const describedBy = `${statusId}-${provider.id}`
              return (
                <NavItem
                  key={provider.id}
                  active={provider.id === selectedId}
                  label={provider.name}
                  icon={<ProviderFrame provider={provider} size={24} />}
                  describedBy={describedBy}
                  onSelect={() => setSelectedId(provider.id)}
                  trailing={
                    <>
                      <span id={describedBy} className="sr-only">
                        {[status, ...roles].join(', ')}
                      </span>
                      {roles.map((role) => (
                        <span key={role} className="kv-tag accent" aria-hidden="true">{role}</span>
                      ))}
                      <span
                        className={`kv-websearch-dot ${ready ? 'ok' : ''}`}
                        title={status}
                        aria-hidden="true"
                      />
                    </>
                  }
                />
              )
            })}
          </div>
        </nav>

        <div className="kv-cli-detail">
          {selected ? (
            <ProviderDetail t={t} provider={selected} config={config} onChange={onChange} />
          ) : (
            <DefaultsPage t={t} config={config} onChange={onChange} onOpenProvider={setSelectedId} />
          )}
        </div>
      </div>
    </div>
  )
}
