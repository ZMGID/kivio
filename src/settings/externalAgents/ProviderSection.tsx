import { useEffect, useState } from 'react'
import { Download, ExternalLink, Pencil, Plus, Terminal, Trash2 } from 'lucide-react'
import {
  externalCliSettingsApi,
  type DetectedExternalAgent,
  type DshOfficialCredential,
  type ImportedCcSwitchProvider,
  type NativeProviderSummary,
} from '../../api/externalCliSettings'
import type { ExternalCliAgentConfig, ExternalCliProvider } from '../../api/tauri'
import { notifyError } from '../../components/appNoticeStore'
import { Button, IconButton } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { i18n, type Lang } from '../../components/i18n'
import { CcSwitchImportModal } from '../CcSwitchImportModal'
import { CliProviderModal } from '../CliProviderModal'
import { dshNativeDetailToProvider } from '../cliNativeProviderConfigs'
import { Input, Toggle } from '../components'
import { Section } from './shared'

const DSH_OFFICIAL_PROVIDER_ID = 'deepseek-official'
const DSH_OFFICIAL_KEY_URL = 'https://platform.deepseek.com/api_keys'

type Strings = (typeof i18n)[Lang]

/**
 * 各 CLI 接第三方供应商的方式不同，界面上的差别都由这里一处决定。
 * - `coexist`：所有已启用的供应商同时可用，`currentProvider` 只是默认项 → 每行一个开关。
 * - 否则同一时间只用一个 → 行首单选。
 */
function providerTraits(agentId: string, t: Strings) {
  const coexist = agentId === 'pi' || agentId === 'opencode' || agentId === 'dsh'
  const nativeOnDisk =
    agentId === 'opencode' || agentId === 'pi' || agentId === 'grok' || agentId === 'kimi'
  const envOnly = agentId !== 'claude' && agentId !== 'codex' && agentId !== 'dsh' && !nativeOnDisk
  return {
    coexist,
    /** dsh 的原生供应商可在这里编辑 / 删除，也不需要「使用 CLI 自身配置」那一行。 */
    ownsNativeProviders: agentId === 'dsh',
    scope: coexist
      ? t.externalAgentsProviderCoexistScope
      : nativeOnDisk
        ? t.externalAgentsNativeScope
        : envOnly
          ? t.externalAgentsProviderEnvOnly
          : t.externalAgentsProviderScope,
  }
}

function withOfficialDshProvider(
  agentId: string,
  natives: NativeProviderSummary[],
): NativeProviderSummary[] {
  if (agentId !== 'dsh') return natives
  if (natives.some((provider) => provider.id === DSH_OFFICIAL_PROVIDER_ID)) return natives
  return [
    {
      id: DSH_OFFICIAL_PROVIDER_ID,
      name: 'DeepSeek',
      modelCount: 2,
      isDefault: natives.every((provider) => !provider.isDefault),
    },
    ...natives,
  ]
}

/** 一个供应商的展示副标题：备注 > base_url > codex 的 model_provider 行。 */
function providerSubtitle(provider: ExternalCliProvider): string {
  if (provider.remark) return provider.remark
  const baseUrl = provider.env?.find((pair) => /BASE_URL$/i.test(pair.key))?.value
  if (baseUrl) return baseUrl
  const fromToml = provider.configToml?.match(/base_url\s*=\s*"([^"]+)"/)?.[1]
  if (fromToml) return fromToml
  try {
    const config = JSON.parse(provider.configJson ?? '{}') as Record<string, unknown>
    if (typeof config.baseUrl === 'string') return config.baseUrl
    if (typeof config.baseURL === 'string') return config.baseURL
    const options = config.options as Record<string, unknown> | undefined
    if (typeof options?.baseURL === 'string') return options.baseURL
  } catch {
    // The edit modal will surface malformed Kivio-owned JSON; keep the list usable meanwhile.
  }
  return ''
}

/**
 * 行首那个小方块里的字。
 *
 * ponytail: 不按 base_url 猜品牌图标（那要一整个图标包 + 一张域名映射表，
 * 而中转站域名千奇百怪，猜错比不猜更糟）。取首个字母/汉字就够把几十条区分开。
 */
function monogram(name: string): string {
  const first = name.trim().match(/[\p{L}\p{N}]/u)?.[0] ?? '·'
  return first.toUpperCase()
}

function ProviderRadio({
  checked,
  label,
  onSelect,
}: {
  checked: boolean
  label: string
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      aria-label={label}
      className="kv-cli-radio"
      onClick={() => {
        if (!checked) onSelect()
      }}
      data-tauri-drag-region="false"
    />
  )
}

/** 供应商列表里的一行。三种来源（CLI 自身 / 原生 / Kivio 托管）共用同一个骨架。 */
function ProviderRow({
  lead,
  name,
  badge,
  subtitle,
  muted,
  children,
}: {
  lead: React.ReactNode
  name: string
  badge?: { label: string; tone?: 'ok' } | null
  subtitle?: string
  muted?: boolean
  children?: React.ReactNode
}) {
  return (
    <div className={`kv-row kv-cli-provider ${muted ? 'is-off' : ''}`}>
      {lead}
      <div className="kv-row-text">
        <div className="kv-row-label kv-cli-provider-name">
          <span className="truncate">{name}</span>
          {badge && <span className={`kv-tag ${badge.tone ?? ''}`}>{badge.label}</span>}
        </div>
        {subtitle && (
          <p className="kv-row-desc truncate" title={subtitle}>
            {subtitle}
          </p>
        )}
      </div>
      {children && <div className="kv-row-control">{children}</div>}
    </div>
  )
}

function RowActions({
  t,
  onEdit,
  onDelete,
}: {
  t: Strings
  onEdit?: () => void
  onDelete?: () => void
}) {
  return (
    <span className="kv-cli-row-actions">
      {onEdit && (
        <IconButton size="sm" variant="ghost" label={t.externalAgentsProviderEdit} onClick={onEdit}>
          <Pencil size={13} />
        </IconButton>
      )}
      {onDelete && (
        <IconButton size="sm" variant="ghost" label={t.externalAgentsRemove} onClick={onDelete}>
          <Trash2 size={13} />
        </IconButton>
      )}
    </span>
  )
}

function DshOfficialKeyCard({ lang }: { lang: Lang }) {
  const t = i18n[lang]
  const [status, setStatus] = useState<DshOfficialCredential | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void externalCliSettingsApi
      .dshOfficialCredentialStatus()
      .then((next) => {
        if (!cancelled) setStatus(next)
      })
      .catch(() => {
        if (!cancelled) setStatus({ configured: false, writable: true })
      })
    return () => {
      cancelled = true
    }
  }, [])

  const writable = status?.writable ?? true
  const save = async () => {
    const key = apiKey.trim()
    if (!key || !writable) return
    setSaving(true)
    setError(null)
    try {
      setStatus(await externalCliSettingsApi.dshOfficialCredentialSave(key))
      setApiKey('')
    } catch (err) {
      setError(String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="kv-cli-card kv-dsh-official-key">
      <div className="kv-dsh-official-key-head">
        <label htmlFor="dsh-official-key" className="kv-row-label">
          {t.externalAgentsDshOfficialKey}
        </label>
        {status && !status.configured && writable && (
          <span className="kv-tag warn">{t.externalAgentsDshOfficialKeyUnsetTag}</span>
        )}
        {status?.configured && (
          <span className="kv-tag ok">{t.externalAgentsDshOfficialKeySetTag}</span>
        )}
        <a
          className="kv-cli-link"
          href={DSH_OFFICIAL_KEY_URL}
          target="_blank"
          rel="noreferrer"
          data-tauri-drag-region="false"
        >
          <ExternalLink size={12} />
          {t.externalAgentsDshOfficialKeyGet}
        </a>
      </div>
      <div className="kv-dsh-official-key-field">
        <Input
          id="dsh-official-key"
          type="password"
          value={apiKey}
          onChange={setApiKey}
          placeholder={
            status?.configured
              ? t.externalAgentsDshOfficialKeySet
              : t.externalAgentsDshOfficialKeyUnset
          }
          mono
          disabled={!writable}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void save()
          }}
        />
        {writable && (
          <Button
            size="sm"
            variant="primary"
            disabled={saving || !apiKey.trim()}
            onClick={() => void save()}
          >
            {saving ? t.externalAgentsDshOfficialKeySaving : t.externalAgentsDshOfficialKeySave}
          </Button>
        )}
      </div>
      <p className="kv-row-desc">
        {writable ? t.externalAgentsDshOfficialKeyHint : t.externalAgentsDshOfficialKeyLocked}
      </p>
      {error && <p className="kv-row-desc kv-dsh-field-error">{error}</p>}
    </div>
  )
}

/**
 * 「供应商」分区：列表 + 导入 + 添加。
 *
 * 只改设置草稿，落地由后端在保存设置时统一做（`persist_settings` → `materialize_all`），
 * 所以这里不需要在每个操作后再调一次 apply 命令。
 */
export function ProviderSection({
  lang,
  agentId,
  agentName,
  providers,
  nativeProviders,
  current,
  onPatch,
  reloadAgents,
}: {
  lang: Lang
  agentId: string
  agentName: string
  providers: ExternalCliProvider[]
  nativeProviders: NonNullable<DetectedExternalAgent['nativeProviders']>
  current: string
  onPatch: (patch: Partial<ExternalCliAgentConfig>) => void
  reloadAgents: (force?: boolean) => Promise<void>
}) {
  const t = i18n[lang]
  const traits = providerTraits(agentId, t)
  const [editing, setEditing] = useState<ExternalCliProvider | null | undefined>(undefined)
  const [importing, setImporting] = useState(false)
  const adoptedNativeIds = new Set(
    providers
      .map((provider) => provider.nativeProviderId?.trim())
      .filter((id): id is string => Boolean(id)),
  )
  const listedNatives = withOfficialDshProvider(agentId, nativeProviders).filter(
    (provider) => provider.id === DSH_OFFICIAL_PROVIDER_ID || !adoptedNativeIds.has(provider.id),
  )
  const usingCliConfig = !current

  const save = (provider: ExternalCliProvider) => {
    const existing = providers.find((item) => item.id === provider.id)
    const saved = {
      ...provider,
      disabled: provider.disabled ?? existing?.disabled ?? false,
    }
    const adoptInUse =
      agentId === 'dsh' &&
      !current &&
      !existing &&
      !saved.disabled &&
      Boolean(saved.nativeProviderId) &&
      nativeProviders.some((native) => native.id === saved.nativeProviderId && native.isDefault)
    onPatch({
      providers: existing
        ? providers.map((item) => (item.id === saved.id ? saved : item))
        : [...providers, saved],
      ...(adoptInUse ? { currentProvider: saved.id } : {}),
    })
    setEditing(undefined)
  }

  const setProviderEnabled = (provider: ExternalCliProvider, enabled: boolean) => {
    onPatch({
      providers: providers.map((item) =>
        item.id === provider.id ? { ...item, disabled: !enabled } : item,
      ),
      ...(!enabled && current === provider.id ? { currentProvider: '' } : {}),
    })
  }

  const remove = async (provider: ExternalCliProvider) => {
    const deletesNativeFile = Boolean(
      provider.nativeProviderId &&
      nativeProviders.some((native) => native.id === provider.nativeProviderId),
    )
    const confirmText = deletesNativeFile
      ? t.externalAgentsDshNativeDeleteConfirm
      : t.externalAgentsProviderDeleteConfirm
    if (!(await confirmDialog({
      message: confirmText.replace('{name}', () => provider.name),
      confirmLabel: t.dialogDelete,
      danger: true,
    }))) return
    onPatch({
      providers: providers.filter((p) => p.id !== provider.id),
      ...(current === provider.id ? { currentProvider: '' } : {}),
    })
  }

  const editNative = async (native: NativeProviderSummary) => {
    try {
      const detail = await externalCliSettingsApi.dshNativeProviderGet(native.id)
      setEditing(dshNativeDetailToProvider(detail))
    } catch (err) {
      void notifyError(err instanceof Error ? err.message : String(err))
    }
  }

  const removeNative = async (native: NativeProviderSummary) => {
    if (!(await confirmDialog({
      message: t.externalAgentsDshNativeDeleteConfirm.replace('{name}', () => native.name),
      confirmLabel: t.dialogDelete,
      danger: true,
    }))) return
    try {
      await externalCliSettingsApi.dshNativeProviderDelete(native.id)
    } catch (err) {
      void notifyError(err instanceof Error ? err.message : String(err))
      return
    }
    await reloadAgents(true)
  }

  const importFromCcSwitch = (items: ImportedCcSwitchProvider[]) => {
    // 保留 cc-switch 的原 id：同一条再导一次是更新，不会堆出重复项。
    const merged = [...providers]
    for (const item of items) {
      const idx = merged.findIndex((provider) => provider.id === item.id)
      const next: ExternalCliProvider = {
        id: item.id,
        name: item.name,
        remark: item.remark,
        env: item.env,
        configToml: item.configToml,
        authJson: item.authJson,
        disabled: idx >= 0 ? merged[idx].disabled : false,
      }
      if (idx >= 0) merged[idx] = next
      else merged.push(next)
    }
    onPatch({ providers: merged })
    setImporting(false)
  }

  const nativeRow = (native: NativeProviderSummary) => {
    const official = native.id === DSH_OFFICIAL_PROVIDER_ID
    const editable = traits.ownsNativeProviders && !official
    const inUse = usingCliConfig && native.isDefault
    const models =
      native.modelCount > 0
        ? t.externalAgentsNativeProviderModels.replace('{count}', String(native.modelCount))
        : null
    const subtitle = official
      ? [t.externalAgentsDshOfficialProvider, models].filter(Boolean).join(' · ')
      : [native.id, native.baseUrl, models].filter(Boolean).join(' · ')
    const badge = inUse && traits.coexist
      ? { label: t.externalAgentsProviderDefault, tone: 'ok' as const }
      : native.isDefault
        ? { label: t.externalAgentsNativeProviderDefault, tone: inUse ? ('ok' as const) : undefined }
        : official
          ? null
          : { label: t.externalAgentsNativeProviderBadge }
    return (
      <ProviderRow
        key={`native-${native.id}`}
        lead={<span className="kv-cli-monogram">{monogram(native.name)}</span>}
        name={native.name}
        badge={badge}
        subtitle={subtitle}
      >
        {traits.coexist && current && native.isDefault && (
          <Button size="sm" variant="ghost" onClick={() => onPatch({ currentProvider: '' })}>
            {t.externalAgentsProviderSetDefault}
          </Button>
        )}
        {editable && (
          <RowActions
            t={t}
            onEdit={() => void editNative(native)}
            onDelete={() => void removeNative(native)}
          />
        )}
      </ProviderRow>
    )
  }

  const managedRow = (provider: ExternalCliProvider) => {
    const isCurrent = current === provider.id
    const off = Boolean(provider.disabled)
    return (
      <ProviderRow
        key={provider.id}
        lead={
          traits.coexist ? (
            <span className="kv-cli-monogram">{monogram(provider.name)}</span>
          ) : (
            <ProviderRadio
              checked={isCurrent}
              label={provider.name}
              onSelect={() => onPatch({ currentProvider: provider.id })}
            />
          )
        }
        name={provider.name}
        badge={
          traits.coexist && isCurrent && !off
            ? { label: t.externalAgentsProviderDefault, tone: 'ok' }
            : null
        }
        subtitle={providerSubtitle(provider)}
        muted={traits.coexist && off}
      >
        {traits.coexist && !off && !isCurrent && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onPatch({ currentProvider: provider.id })}
          >
            {t.externalAgentsProviderSetDefault}
          </Button>
        )}
        {traits.coexist && (
          <Toggle
            checked={!off}
            onChange={(enabled) => setProviderEnabled(provider, enabled)}
            ariaLabel={`${provider.name} ${t.externalAgentsEnable}`}
          />
        )}
        <RowActions
          t={t}
          onEdit={() => setEditing(provider)}
          onDelete={() => void remove(provider)}
        />
      </ProviderRow>
    )
  }

  const empty = providers.length === 0 && listedNatives.length === 0

  return (
    <Section
      title={t.externalAgentsProviderAll}
      desc={traits.scope}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={() => setImporting(true)}>
            <Download size={12} />
            {t.externalAgentsProviderImport}
          </Button>
          <Button size="sm" onClick={() => setEditing(null)}>
            <Plus size={12} />
            {t.externalAgentsProviderAdd}
          </Button>
        </>
      }
    >
      {agentId === 'dsh' && <DshOfficialKeyCard lang={lang} />}
      {empty ? (
        // 一条供应商都没有时，「使用 CLI 自身配置」是唯一可能的状态：直接说出来，
        // 不把它渲染成一个只有一项的单选列表。
        <div className="kv-cli-empty">
          <Terminal size={14} aria-hidden="true" />
          <p>{t.externalAgentsProviderEmpty}</p>
        </div>
      ) : (
        <div
          className="kv-cli-card"
          role={traits.coexist ? undefined : 'radiogroup'}
          aria-label={t.externalAgentsProviderAll}
        >
          {traits.ownsNativeProviders ? (
            listedNatives.map(nativeRow)
          ) : (
            <ProviderRow
              lead={
                traits.coexist ? (
                  <span className="kv-cli-monogram">
                    <Terminal size={13} />
                  </span>
                ) : (
                  <ProviderRadio
                    checked={usingCliConfig}
                    label={t.externalAgentsProviderNone}
                    onSelect={() => onPatch({ currentProvider: '' })}
                  />
                )
              }
              name={t.externalAgentsProviderNone}
              badge={
                traits.coexist && usingCliConfig
                  ? { label: t.externalAgentsProviderDefault, tone: 'ok' }
                  : null
              }
              subtitle={t.externalAgentsProviderNoneDesc}
            >
              {traits.coexist && !usingCliConfig && (
                <Button size="sm" variant="ghost" onClick={() => onPatch({ currentProvider: '' })}>
                  {t.externalAgentsProviderSetDefault}
                </Button>
              )}
            </ProviderRow>
          )}
          {providers.map(managedRow)}
          {!traits.ownsNativeProviders && listedNatives.map(nativeRow)}
        </div>
      )}
      {editing !== undefined && (
        <CliProviderModal
          lang={lang}
          agentId={agentId}
          agentName={agentName}
          initial={editing}
          onSave={save}
          onClose={() => setEditing(undefined)}
        />
      )}
      {importing && (
        <CcSwitchImportModal
          lang={lang}
          agentId={agentId}
          existingIds={providers.map((p) => p.id)}
          onImport={importFromCcSwitch}
          onClose={() => setImporting(false)}
        />
      )}
    </Section>
  )
}
