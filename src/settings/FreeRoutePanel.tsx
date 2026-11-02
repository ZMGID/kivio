import { Eye, EyeOff } from 'lucide-react'
import { api, singleVendorApiKey, type FreeRouteConfig, type FreeRouteVendor } from '../api/tauri'
import { IconButton } from '../components/Button'
import type { Lang } from '../components/i18n'
import { FieldBlock, Input, SettingsGroup, Toggle } from './components'
import { FREEROUTE_VENDOR_META } from './freeRouteCatalog'

function vendorRow(config: FreeRouteConfig, id: string): FreeRouteVendor {
  return config.vendors.find((vendor) => vendor.id === id) ?? { id, enabled: true, apiKey: '' }
}

export function FreeRoutePanel({
  config,
  lang,
  revealedKeys,
  onChange,
  onToggleKeyReveal,
}: {
  config: FreeRouteConfig
  lang: Lang
  revealedKeys: Set<string>
  onChange: (next: FreeRouteConfig) => void
  onToggleKeyReveal: (keyId: string) => void
}) {
  const updateVendor = (id: string, patch: Partial<FreeRouteVendor>) => {
    const known = new Set<string>(FREEROUTE_VENDOR_META.map((meta) => meta.id))
    const byId = new Map(config.vendors.filter((vendor) => known.has(vendor.id)).map((vendor) => [vendor.id, vendor]))
    for (const meta of FREEROUTE_VENDOR_META) {
      if (!byId.has(meta.id)) byId.set(meta.id, { id: meta.id, enabled: true, apiKey: '' })
    }
    const current = byId.get(id) ?? { id, enabled: true, apiKey: '' }
    byId.set(id, { ...current, ...patch, id, apiKey: patch.apiKey !== undefined ? singleVendorApiKey(patch.apiKey) : current.apiKey })
    onChange({ vendors: FREEROUTE_VENDOR_META.map((meta) => byId.get(meta.id)!) })
  }

  return (
    <SettingsGroup title={lang === 'zh' ? '免费上游' : 'Free upstreams'}>
      <p className="text-sm opacity-70">
        {lang === 'zh'
          ? '聚合各家官方免费档，按上游自动故障转移。每个厂商只保存一把 Key，存在现有设置密钥存储里。'
          : 'Aggregates official free tiers and fails over across vendors. One key per vendor, stored with existing provider credentials.'}
      </p>
      {FREEROUTE_VENDOR_META.map((meta) => {
        const vendor = vendorRow(config, meta.id)
        const keyId = `freeroute-${meta.id}`
        const revealed = revealedKeys.has(keyId)
        return (
          <div key={meta.id} className="space-y-2 rounded-lg bg-neutral-900/[0.02] p-3 ring-1 ring-neutral-900/[0.06]">
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">{meta.name}</div>
                <button
                  type="button"
                  onClick={() => void api.openExternal(meta.signupUrl)}
                  className="inline-flex w-fit items-center text-[12px] text-indigo-500 hover:underline dark:text-indigo-300"
                  data-tauri-drag-region="false"
                >
                  {lang === 'zh' ? '申请 / 文档 ↗' : 'Sign up / docs ↗'}
                </button>
              </div>
              <Toggle
                checked={vendor.enabled}
                onChange={(enabled) => updateVendor(meta.id, { enabled })}
                ariaLabel={lang === 'zh' ? `启用 ${meta.name}` : `Enable ${meta.name}`}
              />
            </div>
            {meta.allowsAnonymous ? (
              <p className="text-sm opacity-70">
                {lang === 'zh' ? '官方匿名免费档，无需 API Key。' : 'Official anonymous free tier; no API key required.'}
              </p>
            ) : (
              <FieldBlock label={lang === 'zh' ? 'API Key（一把）' : 'API key (one)'}>
                <div className="flex items-center gap-1.5">
                  <Input
                    className="min-w-0 flex-1"
                    type={revealed ? 'text' : 'password'}
                    value={vendor.apiKey}
                    onChange={(apiKey) => updateVendor(meta.id, { apiKey })}
                    placeholder={lang === 'zh' ? '只保存一把 Key' : 'Exactly one key'}
                    mono
                  />
                  <IconButton
                    size="xs"
                    onClick={() => onToggleKeyReveal(keyId)}
                    label={revealed ? (lang === 'zh' ? '隐藏' : 'Hide') : (lang === 'zh' ? '显示' : 'Show')}
                  >
                    {revealed ? <EyeOff size={14} /> : <Eye size={14} />}
                  </IconButton>
                </div>
              </FieldBlock>
            )}
          </div>
        )
      })}
    </SettingsGroup>
  )
}
