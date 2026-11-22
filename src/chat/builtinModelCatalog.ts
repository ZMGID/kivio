import { useCallback, useEffect, useMemo, useState, type RefObject } from 'react'
import { type ModelProvider } from '../api/tauri'
import { getSettingsCached, setFavoriteModelsCached, subscribeSettings } from '../api/settingsCache'
import { isProviderEnabled } from '../settings/public/providers'
import { createProviderRequestDraft } from '../settings/public/providerDraft'

/** 收藏键：providerId 无冒号，model 可能含冒号 → 只按首个冒号切分。 */
export const favKey = (providerId: string, model: string) => `${providerId}:${model}`
/** 可选模型超过这个数才显示搜索框，少量模型时不占空间。 */
export const SEARCH_THRESHOLD = 8

const parseFavKey = (key: string): { providerId: string; model: string } | null => {
  const idx = key.indexOf(':')
  if (idx <= 0 || idx >= key.length - 1) return null
  return { providerId: key.slice(0, idx), model: key.slice(idx + 1) }
}

export interface BuiltinModelCatalog {
  providers: ModelProvider[]
  visibleProviders: { provider: ModelProvider; models: string[] }[]
  currentProvider: ModelProvider | undefined
  favorites: string[]
  favoriteEntries: { key: string; providerId: string; providerName: string; model: string }[]
  toggleFavorite: (providerId: string, model: string) => void
}

/** 内置服务商与收藏：读设置缓存并订阅更新，收藏乐观写回。 */
export function useBuiltinModelCatalog(currentProviderId: string, currentModel: string): BuiltinModelCatalog {
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [favorites, setFavorites] = useState<string[]>([])

  const loadSettings = useCallback(async () => {
    try {
      const settings = await getSettingsCached()
      setProviders(settings.providers || [])
      setFavorites(settings.favoriteModels || [])
    } catch (err) {
      console.error('Failed to load providers:', err)
      setProviders([
        {
          id: currentProviderId || 'dev-provider',
          name: 'Preview',
          apiKeys: [],
          baseUrl: '',
          availableModels: currentModel ? [currentModel] : ['dev-model'],
          enabledModels: currentModel ? [currentModel] : ['dev-model'],
          enabled: true,
          apiFormat: 'openai_chat',
          request: createProviderRequestDraft(),
        },
      ])
    }
  }, [currentModel, currentProviderId])

  useEffect(() => {
    loadSettings()
    // 设置页自动保存（无保存按钮）与返回聊天视图是并发的：挂载时那次读可能拿到落盘
    // 回包前的旧快照。订阅缓存更新，保存完成后立即拿到新 providers/收藏。
    return subscribeSettings((settings) => {
      setProviders(settings.providers || [])
      setFavorites(settings.favoriteModels || [])
    })
  }, [loadSettings])

  const activeProviders = useMemo(() => providers.filter(isProviderEnabled), [providers])
  // 只显示有可选模型的服务商，避免没配置模型的服务商变成空的分组标题。
  const visibleProviders = useMemo(() => activeProviders
    .map((provider) => ({
      provider,
      models: provider.enabledModels.length > 0 ? provider.enabledModels : provider.availableModels,
    }))
    .filter((entry) => entry.models.length > 0), [activeProviders])
  const currentProvider = activeProviders.find((p) => p.id === currentProviderId)
    ?? providers.find((p) => p.id === currentProviderId)

  // 收藏置顶组：按存储顺序，过滤掉失效的（provider 已删/禁用/模型已不在列表）。
  const favoriteEntries = useMemo(() => {
    return favorites
      .map((key) => {
        const parsed = parseFavKey(key)
        if (!parsed) return null
        const entry = visibleProviders.find((v) => v.provider.id === parsed.providerId)
        if (!entry || !entry.models.includes(parsed.model)) return null
        return { key, providerId: parsed.providerId, providerName: entry.provider.name, model: parsed.model }
      })
      .filter((v): v is { key: string; providerId: string; providerName: string; model: string } => v !== null)
  }, [favorites, visibleProviders])

  const toggleFavorite = useCallback(
    (providerId: string, model: string) => {
      const key = favKey(providerId, model)
      const next = favorites.includes(key)
        ? favorites.filter((k) => k !== key)
        : [...favorites, key]
      const previous = favorites
      setFavorites(next) // 乐观更新
      setFavoriteModelsCached(next).catch((err) => {
        console.error('Failed to save favorite models:', err)
        setFavorites(previous) // 回滚
      })
    },
    [favorites],
  )

  return { providers, visibleProviders, currentProvider, favorites, favoriteEntries, toggleFavorite }
}

/**
 * 菜单里的模型行：外层管悬停洗色，里层按钮管尺寸。内置与外部 CLI 列表共用。
 * 选中项不铺底色，只用字重与行尾对勾表达，避免和悬停的灰条混在一起。
 * `enterTarget`：搜索时回车会选中的那一行，预先铺上悬停色作提示。
 */
export function modelRowClass(enterTarget = false): string {
  // 类名写全：Tailwind 只识别源码里完整出现的字符串，不能拼接。
  return enterTarget
    ? 'group flex w-full items-center rounded-md pr-1 transition-colors bg-[color-mix(in_srgb,var(--text)_5%,transparent)]'
    : 'group flex w-full items-center rounded-md pr-1 transition-colors hover:bg-[color-mix(in_srgb,var(--text)_5%,transparent)] has-[:focus-visible]:bg-[color-mix(in_srgb,var(--text)_5%,transparent)]'
}

export function modelRowButtonClass(selected: boolean): string {
  return `kv-menu-row kv-menu-row--lg min-w-0 flex-1 outline-none ${selected ? 'font-medium text-neutral-900' : 'text-neutral-700'}`
}

/** 打开时（鼠标触发）把焦点放进搜索框；键盘打开由 usePopoverMenu 落到当前项。 */
export function useAutoFocusSearch(enabled: boolean, ref: RefObject<HTMLInputElement>) {
  useEffect(() => {
    if (!enabled) return
    const frame = requestAnimationFrame(() => ref.current?.focus())
    return () => cancelAnimationFrame(frame)
  }, [enabled, ref])
}

/** 打开列表时把当前模型滚到视野中间（当前项在第三组时，一打开就能看到它）。只在挂载时做一次。 */
export function useRevealCurrentRow(ref: RefObject<HTMLElement>) {
  useEffect(() => {
    const current = ref.current?.querySelector<HTMLElement>('[aria-current="true"]')
    current?.scrollIntoView?.({ block: 'center' })
  }, [ref])
}
