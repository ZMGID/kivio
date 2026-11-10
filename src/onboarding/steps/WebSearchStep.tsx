import { useState, type CSSProperties } from 'react'
import { ArrowUpRight, Search } from 'lucide-react'
import { api, type Settings, type WebSearchProviderId } from '../../api/tauri'
import { Input, Toggle } from '../../settings/public/controls'
import type { I18n, Lang } from '../../components/i18n'
import { isWebSearchConfigured, webSearchKeyField } from '../../settings/public/webSearch'
import { OnboardingFormRow } from '../OnboardingFormRow'
import { OnboardingBlock, OnboardingStepFrame } from '../OnboardingStepFrame'

type SearchSource = {
  value: WebSearchProviderId
  label: string
  icon?: string
  placeholder: string
  keyUrl?: string
  /** keyless：公共端点，密钥可选；instance：填写自建实例地址。 */
  kind?: 'keyless' | 'instance'
}

// 引导只收录「填一个值就能用」的搜索源；MCP 端点、搜索深度、自定义网址等进阶项留在设置页。
const SEARCH_SOURCES: SearchSource[] = [
  { value: 'keenable', label: 'Keenable', placeholder: 'optional', keyUrl: 'https://keenable.ai', kind: 'keyless' },
  { value: 'tavily', label: 'Tavily', icon: '/search-icons/tavily.png', placeholder: 'tvly-...', keyUrl: 'https://app.tavily.com/home' },
  { value: 'exa', label: 'Exa', icon: '/search-icons/exa.png', placeholder: 'exa-...', keyUrl: 'https://dashboard.exa.ai/api-keys' },
  { value: 'brave', label: 'Brave', icon: '/search-icons/brave.svg', placeholder: 'BSA...', keyUrl: 'https://api-dashboard.search.brave.com/app/keys' },
  { value: 'serper', label: 'Serper', icon: '/search-icons/serper.png', placeholder: 'serper key', keyUrl: 'https://serper.dev/api-key' },
  { value: 'bocha', label: 'Bocha', icon: '/search-icons/bocha.png', placeholder: 'bocha key', keyUrl: 'https://open.bochaai.com' },
  { value: 'zhipu', label: 'Zhipu', icon: '/search-icons/zhipu.png', placeholder: 'zhipu key', keyUrl: 'https://open.bigmodel.cn/usercenter/apikeys' },
  { value: 'kimi', label: 'Kimi', icon: '/search-icons/kimi.svg', placeholder: 'sk-...', keyUrl: 'https://www.kimi.com/code/console' },
  { value: 'deepseek', label: 'DeepSeek', icon: '/search-icons/deepseek.svg', placeholder: 'sk-...', keyUrl: 'https://platform.deepseek.com/api_keys' },
  { value: 'grok', label: 'Grok', icon: '/search-icons/grok.png', placeholder: 'xai-...', keyUrl: 'https://console.x.ai' },
  { value: 'tinyfish', label: 'TinyFish', icon: '/search-icons/tinyfish.png', placeholder: 'tinyfish key', keyUrl: 'https://agent.tinyfish.ai/api-keys' },
  { value: 'ollama', label: 'Ollama', icon: '/search-icons/ollama.png', placeholder: 'ollama key', keyUrl: 'https://ollama.com/settings/keys' },
  { value: 'searxng', label: 'SearXNG', icon: '/search-icons/searxng.svg', placeholder: 'https://searx.example', kind: 'instance' },
]

type WebSearchStepProps = {
  t: I18n
  lang: Lang
  settings: Settings
  onChange: (settings: Settings) => void
}

function SourceMark({ source, size = 18 }: { source: SearchSource; size?: number }) {
  const [failed, setFailed] = useState(false)
  if (source.icon && !failed) {
    return <img src={source.icon} alt="" width={size} height={size} draggable={false} className="ob-source-img" onError={() => setFailed(true)} />
  }
  return <span className="ob-source-letter" style={{ width: size, height: size }}>{source.label.slice(0, 1)}</span>
}

/** 「联网」插画：搜索框逐字输入，结果卡片带着来源依次落下，最后回答引用来源编号。 */
function SearchArt({ t, source, live }: { t: I18n; source: SearchSource; live: boolean }) {
  return (
    <div className={`ob-search${live ? ' is-live' : ''}`}>
      <div className="ob-search-bar">
        <Search size={14} />
        <span className="ob-search-query"><span>{t.onboardingWebSearchDemoQuery}</span></span>
        <span className="ob-search-engine"><SourceMark source={source} size={14} />{source.label}</span>
      </div>
      <div className="ob-search-results">
        {[0, 1, 2].map((index) => (
          <div key={index} className="ob-search-hit" style={{ '--r': index } as CSSProperties}>
            <span className="ob-search-fav" />
            <span className="ob-search-lines"><i /><i /></span>
            <span className="ob-search-ref">{index + 1}</span>
          </div>
        ))}
      </div>
      <div className="ob-search-answer">
        <span className="ob-search-answer-lines"><i /><i /></span>
        <span className="ob-search-cite">[1]</span>
        <span className="ob-search-cite">[3]</span>
      </div>
    </div>
  )
}

export function WebSearchStep({ t, lang, settings, onChange }: WebSearchStepProps) {
  const webSearch = settings.lens?.webSearch ?? {
    enabled: false,
    provider: 'tavily' as const,
    tavilyApiKey: '',
    exaApiKey: '',
    maxResults: 5,
    searchDepth: 'basic' as const,
  }
  const chatWebSearchEnabled = settings.chatTools.nativeTools?.webSearch !== false
  const source = SEARCH_SOURCES.find((item) => item.value === webSearch.provider) ?? SEARCH_SOURCES[0]
  const listed = SEARCH_SOURCES.some((item) => item.value === webSearch.provider)
  const keyField = source.kind === 'instance' ? null : webSearchKeyField(source.value)
  const fieldValue = source.kind === 'instance'
    ? String(webSearch.searxngBaseUrl ?? '')
    : keyField ? String(webSearch[keyField] ?? '') : ''

  const updateWebSearch = (updates: Partial<NonNullable<Settings['lens']['webSearch']>>) => {
    onChange({
      ...settings,
      lens: {
        ...settings.lens,
        webSearch: {
          ...webSearch,
          ...updates,
        },
      },
    })
  }

  const updateChatWebSearch = (enabled: boolean) => {
    onChange({
      ...settings,
      chatTools: {
        ...settings.chatTools,
        nativeTools: {
          ...settings.chatTools.nativeTools,
          webSearch: enabled,
        },
      },
    })
  }

  const ready = isWebSearchConfigured(webSearch)
  const fieldLabel = source.kind === 'instance'
    ? t.onboardingWebSearchBaseUrl
    : source.kind === 'keyless' ? t.onboardingWebSearchOptionalKey : t.lensWebSearchApiKey

  return (
    <OnboardingStepFrame
      index={3}
      eyebrow={t.onboardingStepWebSearch}
      title={t.onboardingWebSearchTitle}
      subtitle={t.onboardingWebSearchDesc}
      art={<SearchArt t={t} source={source} live={ready} />}
    >
      <OnboardingBlock order={0} label={t.onboardingWebSearchSource}>
        <div className="ob-sources" role="radiogroup" aria-label={t.onboardingWebSearchSource}>
          {SEARCH_SOURCES.map((item, index) => {
            const selected = listed && item.value === webSearch.provider
            return (
              <button
                key={item.value}
                type="button"
                role="radio"
                aria-checked={selected}
                className={`ob-source${selected ? ' is-selected' : ''}`}
                style={{ '--k': index } as CSSProperties}
                onClick={() => updateWebSearch({ provider: item.value })}
                data-tauri-drag-region="false"
              >
                <SourceMark source={item} />
                <span className="ob-source-name">{item.label}</span>
                {item.kind === 'keyless' ? <span className="ob-source-tag">{t.onboardingWebSearchKeyless}</span> : null}
                {item.kind === 'instance' ? <span className="ob-source-tag">{t.onboardingWebSearchSelfHosted}</span> : null}
              </button>
            )
          })}
        </div>

        {listed && (keyField || source.kind === 'instance') ? (
          <OnboardingFormRow
            label={fieldLabel}
            hint={!ready ? t.onboardingWebSearchKeyRequired : source.kind === 'keyless' ? t.onboardingWebSearchKeylessHint : undefined}
            stack
            extra={source.keyUrl && source.kind !== 'keyless' ? (
              <button
                type="button"
                className="ob-link"
                onClick={() => void api.openExternal(source.keyUrl!)}
                data-tauri-drag-region="false"
              >
                {lang === 'zh' ? `获取 ${source.label} API Key` : `Get a ${source.label} API key`}
                <ArrowUpRight size={12} />
              </button>
            ) : undefined}
          >
            <Input
              type={source.kind === 'instance' ? 'text' : 'password'}
              value={fieldValue}
              onChange={(value) => updateWebSearch(source.kind === 'instance'
                ? { searxngBaseUrl: value }
                : keyField ? { [keyField]: value } : {})}
              placeholder={source.placeholder}
              mono
            />
          </OnboardingFormRow>
        ) : null}
      </OnboardingBlock>

      <OnboardingBlock order={1} label={t.onboardingWebSearchEnableSection}>
        <OnboardingFormRow label={t.webSearchChatSection} hint={t.onboardingWebSearchChatHint}>
          <Toggle
            checked={ready && chatWebSearchEnabled}
            disabled={!ready}
            ariaLabel={t.webSearchChatSection}
            onChange={(enabled) => {
              if (!ready) return
              updateChatWebSearch(enabled)
            }}
          />
        </OnboardingFormRow>
        <OnboardingFormRow label={t.webSearchLensSection} hint={t.lensWebSearchHint}>
          <Toggle
            checked={ready && webSearch.enabled}
            disabled={!ready}
            ariaLabel={t.webSearchLensSection}
            onChange={(enabled) => {
              if (!ready) return
              updateWebSearch({ enabled })
            }}
          />
        </OnboardingFormRow>
      </OnboardingBlock>
    </OnboardingStepFrame>
  )
}
