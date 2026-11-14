import { providerAuthenticationReady } from '../../api/tauri'
import { ArrowDownAZ, ArrowLeft, Heart, Plus, RefreshCw, Trash2, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Toggle, Input, SearchInput } from '../components'
import { Button, IconButton } from '../../components/Button'
import { ProviderSortableList } from '../ProviderSortableList'
import { ProviderIcon, PROVIDER_PICKER_KEYS } from '../../components/ModelIcon'
import { PROVIDER_PRESETS, type ProviderPreset } from '../providerPresets'
import { ProviderDetail } from './ProviderDetail'
import { isProviderEnabled } from '../utils'
import type { I18n, Lang } from '../../components/i18n'
import type {
  Settings as SettingsData,
  ModelProvider,
} from '../../api/tauri'

interface ProvidersTabProps {
  settings: SettingsData
  t: I18n
  lang: Lang
  /** 外壳当前选中的供应商；新增后外壳会把它指向新建的那一条。 */
  selectedProvider: ModelProvider | undefined
  revealedKeys: Set<string>
  gzipInfoOpen: Set<string>
  onSelectProvider: (id: string) => void
  onReorderProviders: (fromId: string, toId: string) => void
  onAddProvider: () => void
  onAddProviderFromPreset: (preset: ProviderPreset) => void
  onUpdateProvider: (id: string, updates: Partial<ModelProvider>) => void
  onSetProviderIcon: (id: string, dataUrl: string) => void
  onRequestDeleteProvider: (id: string) => void
  onToggleGzipInfo: (id: string) => void
  onToggleKeyReveal: (keyId: string) => void
  onOpenModelPicker: (id: string) => void
  onOpenModelTest: (id: string) => void
  onOpenModelDrawer: (target: { providerId: string; model: string }) => void
  onRemoveEnabledModel: (providerId: string, model: string) => void
}

function matchesQuery(query: string, ...fields: string[]): boolean {
  return fields.some((field) => field.toLowerCase().includes(query))
}

/**
 * 供应商页。两级：
 *   - 列表：「已添加」的供应商卡片；「添加供应商」打开添加弹窗，在里面选预设或自定义；
 *   - 详情：点开一张卡片后编辑这家供应商（连接、模型、高级）。
 * 纯展示：设置草稿和弹窗状态留在 SettingsShell，这里只记「当前点开的是谁」。
 */
export function ProvidersTab({
  settings,
  t,
  lang,
  selectedProvider,
  revealedKeys,
  gzipInfoOpen,
  onSelectProvider,
  onReorderProviders,
  onAddProvider,
  onAddProviderFromPreset,
  onUpdateProvider,
  onSetProviderIcon,
  onRequestDeleteProvider,
  onToggleGzipInfo,
  onToggleKeyReveal,
  onOpenModelPicker,
  onOpenModelTest,
  onOpenModelDrawer,
  onRemoveEnabledModel,
}: ProvidersTabProps) {
  const zh = lang === 'zh'
  const [openId, setOpenId] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [iconPickerOpen, setIconPickerOpen] = useState(false)
  const [requestPageOpen, setRequestPageOpen] = useState(false)
  const [addOpen, setAddOpen] = useState(false)
  const [presetQuery, setPresetQuery] = useState('')
  const [presetSortAz, setPresetSortAz] = useState(false)
  // 新增后直接进详情填密钥。新 id 由外壳生成，这里记下新增前已有的 id，等外壳选中一个不在其中的。
  const idsBeforeAdd = useRef<Set<string> | null>(null)

  // 被删除的供应商查不到，自然退回列表。
  const opened = openId ? settings.providers.find((provider) => provider.id === openId) : undefined

  useEffect(() => {
    const before = idsBeforeAdd.current
    if (!before || !selectedProvider || before.has(selectedProvider.id)) return
    idsBeforeAdd.current = null
    setOpenId(selectedProvider.id)
  }, [selectedProvider])

  // 换一家供应商时收起图标选择器、退出二级页：否则会悄悄改到另一家头上。
  useEffect(() => {
    setIconPickerOpen(false)
    setRequestPageOpen(false)
  }, [openId])

  const open = (id: string) => {
    onSelectProvider(id)
    setOpenId(id)
  }

  const closeAdd = () => {
    setAddOpen(false)
    setPresetQuery('')
    setPresetSortAz(false)
  }

  // 添加后直接进详情填密钥：记下现有 id，等外壳选中新建的那一条（见上面的 effect）。
  const addThenOpen = (add: () => void) => {
    idsBeforeAdd.current = new Set(settings.providers.map((provider) => provider.id))
    add()
    closeAdd()
  }

  const needle = query.trim().toLowerCase()
  const shownProviders = useMemo(
    () => needle
      ? settings.providers.filter((provider) => matchesQuery(needle, provider.name, provider.baseUrl))
      : settings.providers,
    [needle, settings.providers],
  )
  // 添加弹窗里的预设：赞助位始终在前，其余可按名称排序。
  const presetMatches = useMemo(() => {
    const q = presetQuery.trim().toLowerCase()
    const list = q
      ? PROVIDER_PRESETS.filter((preset) => matchesQuery(q, preset.name, preset.baseUrl))
      : PROVIDER_PRESETS
    const rest = list.filter((preset) => !preset.sponsored)
    if (presetSortAz) {
      rest.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
    }
    return [...list.filter((preset) => preset.sponsored), ...rest]
  }, [presetQuery, presetSortAz])

  if (opened) {
    const enabled = isProviderEnabled(opened)
    const configured = providerAuthenticationReady(opened)
    const iconKey = settings.providerIcons?.[opened.id]
    return (
      <div className="kv-provider-page">
        {/* 「请求配置」二级页自带返回，接管整页；这时不再叠一层顶栏。 */}
        {!requestPageOpen && (
          <>
            <button
              type="button"
              onClick={() => setOpenId(null)}
              className="kv-subpage-back"
              data-tauri-drag-region="false"
            >
              <ArrowLeft size={14} />
              <span>{zh ? '全部供应商' : 'All providers'}</span>
            </button>
            {/* 顶栏：这家供应商是谁、通不通、开没开。配置项留给下面的分区。 */}
            <header className="kv-provider-hero">
              <button
                type="button"
                onClick={() => setIconPickerOpen((v) => !v)}
                title={zh ? '选择图标' : 'Choose icon'}
                aria-label={zh ? '选择图标' : 'Choose icon'}
                aria-expanded={iconPickerOpen}
                data-tauri-drag-region="false"
                className="kv-provider-hero-icon"
              >
                <ProviderIcon name={opened.name} baseUrl={opened.baseUrl} iconKey={iconKey} size={20} />
              </button>
              <Input
                className="kv-provider-hero-name"
                value={opened.name}
                onChange={(v) => onUpdateProvider(opened.id, { name: v })}
                placeholder={t.providerName}
                aria-label={t.providerName}
              />
              <span className={`kv-tag ${enabled && configured ? 'ok' : enabled ? 'warn' : ''}`}>
                {!enabled
                  ? (zh ? '已停用' : 'Disabled')
                  : configured ? t.connectionOk : t.permissionMissing}
              </span>
              <Button
                size="sm"
                onClick={() => onOpenModelTest(opened.id)}
                data-tauri-drag-region="false"
              >
                <RefreshCw size={11} />
                {t.testConnection}
              </Button>
              <Toggle
                checked={enabled}
                onChange={(next) => onUpdateProvider(opened.id, { enabled: next })}
                ariaLabel={zh ? '启用供应商' : 'Enable provider'}
              />
              <IconButton
                variant="danger"
                size="sm"
                onClick={() => onRequestDeleteProvider(opened.id)}
                data-tauri-drag-region="false"
                title={t.deleteProvider}
                label={t.deleteProvider}
              >
                <Trash2 size={13} />
              </IconButton>
            </header>
            {iconPickerOpen && (
              <div className="kv-provider-icon-grid" role="listbox" aria-label={zh ? '选择图标' : 'Choose icon'}>
                <button
                  type="button"
                  role="option"
                  aria-selected={!iconKey}
                  onClick={() => {
                    onSetProviderIcon(opened.id, '')
                    setIconPickerOpen(false)
                  }}
                  data-tauri-drag-region="false"
                  className="kv-provider-icon-cell is-text"
                >
                  {zh ? '自动' : 'Auto'}
                </button>
                {PROVIDER_PICKER_KEYS.map((key) => (
                  <button
                    key={key}
                    type="button"
                    role="option"
                    aria-selected={iconKey === key}
                    title={key}
                    onClick={() => {
                      onSetProviderIcon(opened.id, key)
                      setIconPickerOpen(false)
                    }}
                    data-tauri-drag-region="false"
                    className="kv-provider-icon-cell"
                  >
                    <ProviderIcon name={key} iconKey={key} size={18} />
                  </button>
                ))}
              </div>
            )}
          </>
        )}
        <ProviderDetail
          provider={opened}
          t={t}
          lang={lang}
          revealedKeys={revealedKeys}
          gzipInfoOpen={gzipInfoOpen}
          requestPageOpen={requestPageOpen}
          onRequestPageOpenChange={setRequestPageOpen}
          onUpdateProvider={onUpdateProvider}
          onToggleGzipInfo={onToggleGzipInfo}
          onToggleKeyReveal={onToggleKeyReveal}
          onOpenModelPicker={onOpenModelPicker}
          onOpenModelDrawer={onOpenModelDrawer}
          onRemoveEnabledModel={onRemoveEnabledModel}
        />
      </div>
    )
  }

  return (
    <div className="kv-provider-page">
      <div className="kv-provider-toolbar">
        <SearchInput
          className="kv-provider-toolbar-search"
          value={query}
          onChange={setQuery}
          placeholder={t.presetProvidersSearch}
          aria-label={t.presetProvidersSearch}
          clearLabel={t.dialogClear}
        />
        <Button
          variant="primary"
          onClick={() => setAddOpen(true)}
          data-tauri-drag-region="false"
        >
          <Plus size={13} />
          {t.addProvider}
        </Button>
      </div>

      <section className="kv-provider-block" aria-labelledby="kv-provider-added-title">
        <h3 id="kv-provider-added-title" className="kv-provider-block-title">
          {zh ? '已添加' : 'Added'}
          <span className="kv-provider-block-count">{shownProviders.length}</span>
        </h3>
        {shownProviders.length > 0 ? (
          <ProviderSortableList
            providers={shownProviders}
            sortable={!needle && shownProviders.length > 1}
            t={t}
            lang={lang}
            icons={settings.providerIcons}
            onOpen={open}
            onToggleEnabled={(id, enabled) => onUpdateProvider(id, { enabled })}
            onRequestDelete={onRequestDeleteProvider}
            onReorder={onReorderProviders}
          />
        ) : (
          <p className="kv-provider-block-empty">
            {needle
              ? t.presetNoSearchResults
              : (zh ? '还没有供应商。点「添加供应商」选一个常用服务商，或自己填。' : 'No providers yet. Use Add Provider to pick a common service or fill one in yourself.')}
          </p>
        )}
      </section>

      {addOpen && createPortal(
        <div
          className="kv-modal-backdrop kv-modal-backdrop--portal"
          data-tauri-drag-region="false"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) closeAdd()
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation()
              closeAdd()
            }
          }}
        >
          <div
            className="kv kv-modal kv-provider-preset-picker"
            role="dialog"
            aria-modal="true"
            aria-labelledby="kv-provider-add-title"
            data-tauri-drag-region="false"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <div className="kv-provider-preset-picker-header">
              <div className="kv-provider-preset-picker-heading">
                <h3 id="kv-provider-add-title" className="kv-provider-preset-picker-title">
                  {t.addProvider}
                </h3>
                <p className="kv-provider-preset-picker-hint">{t.presetProvidersHint}</p>
              </div>
              <IconButton
                size="xs"
                onClick={closeAdd}
                data-tauri-drag-region="false"
                label={zh ? '关闭' : 'Close'}
              >
                <X size={14} />
              </IconButton>
            </div>
            <div className="kv-provider-preset-picker-toolbar">
              <SearchInput
                className="kv-provider-preset-picker-search"
                value={presetQuery}
                onChange={setPresetQuery}
                placeholder={t.presetProvidersSearch}
                aria-label={t.presetProvidersSearch}
                clearLabel={t.dialogClear}
                autoFocus
              />
              <IconButton
                size="sm"
                className={presetSortAz ? 'is-active' : ''}
                onClick={() => setPresetSortAz((on) => !on)}
                data-tauri-drag-region="false"
                aria-pressed={presetSortAz}
                label={t.presetSortAz}
              >
                <ArrowDownAZ size={14} />
              </IconButton>
            </div>
            <div className="kv-provider-preset-picker-body custom-scrollbar">
              <button
                type="button"
                className="kv-provider-preset-tile is-custom"
                onClick={() => addThenOpen(onAddProvider)}
                data-tauri-drag-region="false"
              >
                <Plus size={16} strokeWidth={2.25} />
                <span className="kv-provider-preset-tile-name">{t.presetCustom}</span>
              </button>
              {presetMatches.map((preset) => (
                <button
                  key={preset.name}
                  type="button"
                  className="kv-provider-preset-tile"
                  title={preset.baseUrl}
                  onClick={() => addThenOpen(() => onAddProviderFromPreset(preset))}
                  data-tauri-drag-region="false"
                >
                  <ProviderIcon name={preset.name} baseUrl={preset.baseUrl} size={18} />
                  <span className="kv-provider-preset-tile-name">{preset.name}</span>
                  {preset.sponsored ? (
                    <Heart
                      size={12}
                      strokeWidth={2}
                      fill="currentColor"
                      className="kv-provider-preset-tile-heart"
                      aria-label={t.presetSponsored}
                    />
                  ) : null}
                </button>
              ))}
              {presetMatches.length === 0 && (
                <p className="kv-provider-preset-picker-empty">{t.presetNoSearchResults}</p>
              )}
            </div>
            <p className="kv-provider-preset-picker-foot">{t.presetCustomHint}</p>
          </div>
        </div>,
        document.body,
      )}
    </div>
  )
}
