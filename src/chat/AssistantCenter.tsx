import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  ArrowLeft,
  BookOpen,
  Check,
  Copy,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Save,
  Search,
  Sparkles,
  Trash2,
  Wrench,
} from 'lucide-react'
import { api, type ModelProvider } from '../api/tauri'
import { isProviderEnabled } from '../settings/utils'
import { Select } from '../settings/components'
import { builtinAssistantGlyph } from './assistantIcons'
import { chatApi } from './api'
import { usesNativeTitlebar } from './platform'
import type { ChatAssistant, SkillMeta } from './types'

interface AssistantCenterProps {
  skills: SkillMeta[]
  currentAssistantId?: string | null
  onStartAssistantChat: (assistant: ChatAssistant) => void
  onStartBuilder?: () => void
  onApplyAssistant?: (assistantId: string | null) => void
  onClose: () => void
}

type AssistantDraft = ChatAssistant
type CenterView = 'list' | 'detail' | 'edit'
type SuiteTab = 'plaza' | 'installed' | 'mine'

const assistantColors = ['#6A8FBD', '#C56646', '#4F9D7A', '#8A6FBD', '#B7791F', '#5E8C6A']

function nowSeconds() {
  return Math.floor(Date.now() / 1000)
}

function listFromMaybe<T>(snake?: T[], camel?: T[]): T[] {
  return Array.isArray(snake) ? snake : Array.isArray(camel) ? camel : []
}

function assistantMcpIds(assistant?: ChatAssistant | null): string[] {
  return listFromMaybe(assistant?.mcp_server_ids, assistant?.mcpServerIds)
}

function assistantSkillIds(assistant?: ChatAssistant | null): string[] {
  return listFromMaybe(assistant?.skill_ids, assistant?.skillIds)
}

function toggleId(list: string[], id: string): string[] {
  return list.includes(id) ? list.filter((item) => item !== id) : [...list, id]
}

function normalizeStringList(values?: string[], limit = 64): string[] {
  const out: string[] = []
  for (const value of values ?? []) {
    const item = value.trim()
    if (!item || out.includes(item)) continue
    out.push(item)
    if (out.length >= limit) break
  }
  return out
}

function normalizeAssistantForDraft(assistant: ChatAssistant): AssistantDraft {
  return {
    ...assistant,
    description: assistant.description ?? '',
    icon: assistant.icon ?? 'bot',
    color: assistant.color ?? '#6A8FBD',
    source: assistant.source ?? (assistant.built_in ?? assistant.builtIn ? 'builtin' : 'user'),
    system_prompt: assistant.system_prompt ?? assistant.systemPrompt ?? '',
    provider_id: assistant.provider_id ?? assistant.providerId ?? '',
    model: assistant.model ?? '',
    mcp_server_ids: assistantMcpIds(assistant),
    skill_ids: assistantSkillIds(assistant),
    enabled: assistant.enabled ?? true,
    installed: assistant.installed ?? true,
    archived: assistant.archived ?? false,
    built_in: assistant.built_in ?? assistant.builtIn ?? false,
    created_at: assistant.created_at ?? assistant.createdAt ?? nowSeconds(),
    updated_at: assistant.updated_at ?? assistant.updatedAt ?? nowSeconds(),
  }
}

function createBlankAssistant(): AssistantDraft {
  const now = nowSeconds()
  return {
    id: `asst_${crypto.randomUUID()}`,
    name: '新助手',
    description: '',
    icon: 'bot',
    color: '#6A8FBD',
    source: 'user',
    system_prompt: '',
    provider_id: '',
    model: '',
    mcp_server_ids: [],
    skill_ids: [],
    enabled: true,
    installed: true,
    archived: false,
    built_in: false,
    created_at: now,
    updated_at: now,
  }
}

function draftPayload(draft: AssistantDraft): ChatAssistant {
  return {
    ...draft,
    name: draft.name.trim(),
    description: draft.description?.trim() ?? '',
    icon: draft.icon?.trim() || 'bot',
    color: draft.color?.trim() || '#6A8FBD',
    source: draft.source || (draft.built_in ?? draft.builtIn ? 'builtin' : 'user'),
    system_prompt: (draft.system_prompt ?? draft.systemPrompt ?? '').trim(),
    provider_id: (draft.provider_id ?? draft.providerId ?? '').trim(),
    model: draft.provider_id ? (draft.model ?? '').trim() : '',
    mcp_server_ids: normalizeStringList(assistantMcpIds(draft)),
    skill_ids: normalizeStringList(assistantSkillIds(draft)),
    enabled: draft.enabled ?? true,
    installed: draft.installed ?? true,
    archived: false,
    built_in: draft.built_in ?? draft.builtIn ?? false,
    created_at: draft.created_at,
    updated_at: nowSeconds(),
  }
}

function assistantMatches(assistant: ChatAssistant, query: string) {
  if (!query) return true
  const text = [assistant.name, assistant.description, assistant.system_prompt ?? assistant.systemPrompt]
    .filter(Boolean)
    .join('\n')
    .toLowerCase()
  return text.includes(query)
}

function providerModels(provider?: ModelProvider): string[] {
  if (!provider) return []
  return provider.enabledModels.length > 0 ? provider.enabledModels : provider.availableModels
}

function suiteStats(assistant: ChatAssistant) {
  return {
    mcp: assistantMcpIds(assistant).length,
    skills: assistantSkillIds(assistant).length,
  }
}

export function AssistantCenter({
  skills,
  currentAssistantId,
  onStartAssistantChat,
  onStartBuilder,
  onApplyAssistant,
  onClose,
}: AssistantCenterProps) {
  const [assistants, setAssistants] = useState<ChatAssistant[]>([])
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [mcpServers, setMcpServers] = useState<Array<{ id: string; name: string }>>([])
  const [selectedId, setSelectedId] = useState<string | null>(currentAssistantId ?? null)
  const [draft, setDraft] = useState<AssistantDraft | null>(null)
  const [query, setQuery] = useState('')
  const [view, setView] = useState<CenterView>('list')
  const [tab, setTab] = useState<SuiteTab>('installed')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')

  const loadAssistants = useCallback(async (preferredId?: string | null) => {
    setLoading(true)
    setError('')
    try {
      const data = await chatApi.getAssistants()
      setAssistants(data)
      const nextSelectedId = preferredId ?? currentAssistantId ?? data[0]?.id ?? null
      const selected = data.find((assistant) => assistant.id === nextSelectedId) ?? null
      setSelectedId(selected?.id ?? null)
      setDraft(selected ? normalizeAssistantForDraft(selected) : null)
    } catch (err) {
      setError(typeof err === 'string' ? err : (err as Error).message || '套件載入失敗')
    } finally {
      setLoading(false)
    }
  }, [currentAssistantId])

  const loadProviders = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      setProviders(settings.providers || [])
      setMcpServers(
        (settings.chatTools?.servers ?? []).map((server) => ({ id: server.id, name: server.name })),
      )
    } catch {
      setProviders([])
      setMcpServers([])
    }
  }, [])

  useEffect(() => {
    void loadAssistants(currentAssistantId)
    void loadProviders()
  }, [currentAssistantId, loadAssistants, loadProviders])

  // 對話搭建落庫後會發 chat-assistants-changed,重新整理列表讓新專家即時出現。
  useEffect(() => {
    const unlistenPromise = api.onChatAssistantsChanged(() => void loadAssistants())
    return () => {
      void unlistenPromise.then((unlisten) => unlisten())
    }
  }, [loadAssistants])

  const selectedAssistant = useMemo(
    () => assistants.find((assistant) => assistant.id === selectedId) ?? null,
    [assistants, selectedId],
  )

  const filteredAssistants = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase()
    return assistants.filter((assistant) => {
      if (!assistantMatches(assistant, normalizedQuery)) return false
      const builtIn = assistant.built_in ?? assistant.builtIn ?? false
      if (tab === 'plaza' && !builtIn) return false
      if (tab === 'installed' && assistant.installed === false) return false
      if (tab === 'mine' && builtIn) return false
      return true
    })
  }, [assistants, query, tab])

  const enabledProviders = useMemo(
    () => providers.filter(isProviderEnabled),
    [providers],
  )

  const selectedProvider = providers.find((provider) => provider.id === (draft?.provider_id ?? draft?.providerId))
  const models = providerModels(selectedProvider)
  const draftMcpIds = assistantMcpIds(draft)
  const draftSkillIds = assistantSkillIds(draft)
  const canApplyCurrent = Boolean(onApplyAssistant)
  const builtInCount = assistants.filter((assistant) => assistant.built_in ?? assistant.builtIn).length
  const installedCount = assistants.filter((assistant) => assistant.installed !== false).length

  const updateDraft = <K extends keyof AssistantDraft>(key: K, value: AssistantDraft[K]) => {
    setDraft((prev) => (prev ? { ...prev, [key]: value } : prev))
  }

  const openDetail = (assistant: ChatAssistant) => {
    setSelectedId(assistant.id)
    setDraft(normalizeAssistantForDraft(assistant))
    setView('detail')
    setError('')
  }

  const handleCreate = () => {
    const blank = createBlankAssistant()
    setSelectedId(null)
    setDraft(blank)
    setView('edit')
    setError('')
  }

  const saveDraft = async (): Promise<ChatAssistant | null> => {
    if (!draft) return null
    const payload = draftPayload(draft)
    if (!payload.name) {
      setError('套件名稱不能為空')
      return null
    }
    setSaving(true)
    setError('')
    try {
      const exists = assistants.some((assistant) => assistant.id === payload.id)
      const saved = exists
        ? await chatApi.updateAssistant(payload)
        : await chatApi.createAssistant(payload)
      await loadAssistants(saved.id)
      setSelectedId(saved.id)
      setDraft(normalizeAssistantForDraft(saved))
      return saved
    } catch (err) {
      setError(typeof err === 'string' ? err : (err as Error).message || '套件儲存失敗')
      return null
    } finally {
      setSaving(false)
    }
  }

  const handleDuplicate = async (assistant?: ChatAssistant | null) => {
    const target = assistant ?? draft
    if (!target || !assistants.some((item) => item.id === target.id)) return
    setSaving(true)
    setError('')
    try {
      const copy = await chatApi.duplicateAssistant(target.id)
      await loadAssistants(copy.id)
      setSelectedId(copy.id)
      setDraft(normalizeAssistantForDraft(copy))
      setView('edit')
    } catch (err) {
      setError(typeof err === 'string' ? err : (err as Error).message || '複製失敗')
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async () => {
    if (!draft) return
    const exists = assistants.some((assistant) => assistant.id === draft.id)
    if (!exists) {
      setDraft(null)
      setSelectedId(null)
      setView('list')
      return
    }
    if (!window.confirm(`確定刪除套件「${draft.name}」？已有對話會保留當時的套件快照。`)) return
    setSaving(true)
    setError('')
    try {
      await chatApi.deleteAssistant(draft.id)
      await loadAssistants(null)
      setView('list')
    } catch (err) {
      setError(typeof err === 'string' ? err : (err as Error).message || '刪除失敗')
    } finally {
      setSaving(false)
    }
  }

  const handleStartChat = async (assistant?: ChatAssistant | null) => {
    if (assistant) {
      onStartAssistantChat(assistant)
      return
    }
    const saved = await saveDraft()
    if (saved) onStartAssistantChat(saved)
  }

  const handleApplyAssistant = async (assistant?: ChatAssistant | null) => {
    const target = assistant ?? await saveDraft()
    if (target) onApplyAssistant?.(target.id)
  }

  const renderList = () => (
    <div className="space-y-4">
      <div className="assistant-center-tabs flex min-w-0 items-center gap-1 border-b border-neutral-200 pb-2 dark:border-neutral-800">
          {[
            ['plaza', '套件廣場', builtInCount],
            ['installed', '已安裝', installedCount],
            ['mine', '我的', assistants.length - builtInCount],
          ].map(([value, label, count]) => (
            <button
              key={value}
              type="button"
              onClick={() => setTab(value as SuiteTab)}
              className={`flex h-8 items-center gap-2 rounded-md px-2.5 text-[13px] font-medium transition-colors ${
                tab === value
                  ? 'bg-neutral-100 text-neutral-950 dark:bg-neutral-800 dark:text-neutral-50'
                  : 'text-neutral-500 hover:bg-neutral-50 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              {label}
              <span className="rounded-full bg-white px-1.5 py-0.5 text-[11px] text-neutral-500 dark:bg-neutral-950 dark:text-neutral-400">
                {count}
              </span>
            </button>
          ))}
      </div>

      {loading ? (
        <div className="grid min-h-[220px] place-items-center text-[13px] text-neutral-400">載入中...</div>
      ) : filteredAssistants.length === 0 ? (
        <div className="grid min-h-[220px] place-items-center rounded-md border border-dashed border-neutral-200 text-[13px] text-neutral-400 dark:border-neutral-800">
          沒有匹配的套件
        </div>
      ) : (
        <div className="overflow-hidden rounded-md border border-neutral-200 divide-y divide-neutral-200 dark:border-neutral-800 dark:divide-neutral-800">
          {filteredAssistants.map((assistant) => {
            const stats = suiteStats(assistant)
            const builtIn = assistant.built_in ?? assistant.builtIn ?? false
            return (
              <article
                key={assistant.id}
                className="flex min-w-0 items-center gap-3 px-3 py-2.5 transition-colors hover:bg-neutral-50 dark:hover:bg-neutral-900/60"
              >
                <button
                  type="button"
                  onClick={() => openDetail(assistant)}
                  className="grid size-9 shrink-0 place-items-center rounded-md border border-neutral-200 bg-white text-[15px] font-semibold hover:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-950"
                  style={{ color: assistant.color || '#6A8FBD' }}
                  aria-label={`開啟 ${assistant.name}`}
                >
                  {builtinAssistantGlyph(assistant.id, 20) ?? (assistant.name.trim().slice(0, 1) || '套')}
                </button>
                <button
                  type="button"
                  onClick={() => openDetail(assistant)}
                  className="min-w-0 flex-1 text-left"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="truncate text-[14px] font-semibold text-neutral-950 dark:text-neutral-50">
                      {assistant.name}
                    </span>
                    {builtIn && (
                      <span className="shrink-0 rounded bg-emerald-100 px-1.5 py-0.5 text-[11px] text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">
                        內建
                      </span>
                    )}
                    <span className="truncate text-[12px] font-medium text-neutral-400 dark:text-neutral-500">
                      {builtIn ? '內建' : '自訂'}
                    </span>
                  </div>
                  <p className="mt-0.5 truncate text-[12px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                    {assistant.description || '未設定描述'}
                  </p>
                  <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[11px] text-neutral-400 dark:text-neutral-500">
                    <span className="shrink-0">{stats.mcp} MCP</span>
                    <span className="shrink-0 opacity-50">·</span>
                    <span className="shrink-0">{stats.skills} 技能</span>
                    <span className="ml-auto shrink-0">{assistant.enabled === false ? '已停用' : '可用'}</span>
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => void handleStartChat(assistant)}
                  disabled={assistant.enabled === false}
                  className="grid size-8 shrink-0 place-items-center rounded-md text-neutral-900 hover:bg-neutral-100 disabled:cursor-not-allowed disabled:opacity-40 dark:text-neutral-100 dark:hover:bg-neutral-800"
                  aria-label={`使用 ${assistant.name} 開始聊天`}
                  title="開始聊天"
                >
                  <Plus size={18} />
                </button>
              </article>
            )
          })}
        </div>
      )}
    </div>
  )

  const renderDetail = () => {
    const assistant = selectedAssistant
    if (!assistant) return renderList()
    const usedMcpIds = assistantMcpIds(assistant)
    const usedSkillIds = assistantSkillIds(assistant)
    const mcpNames = usedMcpIds.map((id) => mcpServers.find((s) => s.id === id)?.name ?? id)
    const skillNames = usedSkillIds.map((id) => skills.find((s) => s.id === id)?.name ?? id)
    const systemPrompt = assistant.system_prompt ?? assistant.systemPrompt ?? ''
    return (
      <div className="space-y-7">
        <div className="flex flex-col gap-4 border-b border-neutral-200 pb-5 dark:border-neutral-800 lg:flex-row lg:items-start lg:justify-between">
          <div className="flex min-w-0 gap-4">
            <button
              type="button"
              onClick={() => setView('list')}
              className="mt-1 grid size-8 shrink-0 place-items-center rounded-md text-neutral-500 hover:bg-neutral-100 hover:text-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
              aria-label="返回列表"
            >
              <ArrowLeft size={18} />
            </button>
            <div
              className="grid size-16 shrink-0 place-items-center rounded-md text-[26px] font-semibold text-white"
              style={{ backgroundColor: assistant.color || '#6A8FBD' }}
            >
              {builtinAssistantGlyph(assistant.id, 32) ?? (assistant.name.trim().slice(0, 1) || '助')}
            </div>
            <div className="min-w-0">
              <h2 className="truncate text-[28px] font-semibold tracking-normal text-neutral-950 dark:text-neutral-50">
                {assistant.name}
              </h2>
              <div className="mt-1 text-[13px] font-medium text-neutral-500">
                {assistant.enabled === false ? '已停用' : '可用'}
              </div>
              <p className="mt-6 max-w-5xl text-[16px] leading-8 text-neutral-700 dark:text-neutral-300">
                {assistant.description || '這個助手還沒有描述。'}
              </p>
            </div>
          </div>
          <div className="flex shrink-0 flex-wrap gap-2">
            <button
              type="button"
              onClick={() => {
                setDraft(normalizeAssistantForDraft(assistant))
                setView('edit')
              }}
              className="flex h-9 items-center gap-2 rounded-md bg-neutral-100 px-3 text-[13px] font-medium text-neutral-800 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-100 dark:hover:bg-neutral-700"
            >
              <Pencil size={15} />
              編輯
            </button>
            <button
              type="button"
              onClick={() => void handleDuplicate(assistant)}
              className="grid h-9 w-9 place-items-center rounded-md bg-neutral-100 text-neutral-700 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
              title="複製助手"
              aria-label="複製助手"
            >
              <Copy size={15} />
            </button>
            {canApplyCurrent && (
              <button
                type="button"
                onClick={() => void handleApplyAssistant(assistant)}
                disabled={assistant.enabled === false}
                className="flex h-9 items-center gap-2 rounded-md px-3 text-[13px] font-medium text-neutral-700 hover:bg-neutral-100 disabled:cursor-not-allowed disabled:opacity-40 dark:text-neutral-200 dark:hover:bg-neutral-800"
              >
                <Check size={15} />
                應用到當前對話
              </button>
            )}
            <button
              type="button"
              onClick={() => void handleStartChat(assistant)}
              disabled={assistant.enabled === false}
              className="flex h-9 items-center gap-2 rounded-md bg-neutral-950 px-3 text-[13px] font-medium text-white hover:bg-neutral-700 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-neutral-200"
            >
              <Play size={15} />
              開始聊天
            </button>
          </div>
        </div>

        <section className="space-y-3">
          <h3 className="text-[17px] font-semibold text-neutral-950 dark:text-neutral-50">系統提示詞</h3>
          <div className="rounded-md border border-neutral-200 px-4 py-3 text-[13px] leading-relaxed whitespace-pre-wrap text-neutral-700 dark:border-neutral-800 dark:text-neutral-300">
            {systemPrompt || '未設定系統提示詞。'}
          </div>
        </section>

        <div className="grid gap-6 md:grid-cols-2">
          <section className="space-y-3">
            <h3 className="flex items-center gap-2 text-[17px] font-semibold text-neutral-950 dark:text-neutral-50">
              <Wrench size={16} className="text-neutral-400" />
              MCP <span className="text-neutral-400">({mcpNames.length})</span>
            </h3>
            <div className="flex flex-wrap gap-1.5">
              {mcpNames.length === 0 ? (
                <span className="text-[13px] text-neutral-400">未啟用任何 MCP</span>
              ) : mcpNames.map((name) => (
                <span key={name} className="rounded-md bg-neutral-100 px-2.5 py-1 text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                  {name}
                </span>
              ))}
            </div>
          </section>

          <section className="space-y-3">
            <h3 className="flex items-center gap-2 text-[17px] font-semibold text-neutral-950 dark:text-neutral-50">
              <BookOpen size={16} className="text-neutral-400" />
              技能 <span className="text-neutral-400">({skillNames.length})</span>
            </h3>
            <div className="flex flex-wrap gap-1.5">
              {skillNames.length === 0 ? (
                <span className="text-[13px] text-neutral-400">未啟用任何技能</span>
              ) : skillNames.map((name) => (
                <span key={name} className="rounded-md bg-neutral-100 px-2.5 py-1 text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                  {name}
                </span>
              ))}
            </div>
          </section>
        </div>
      </div>
    )
  }

  const renderEdit = () => {
    if (!draft) return renderList()
    return (
      <div className="space-y-6">
        <div className="flex flex-col gap-4 border-b border-neutral-200 pb-4 dark:border-neutral-800 lg:flex-row lg:items-center lg:justify-between">
          <div className="flex min-w-0 items-center gap-3">
            <button
              type="button"
              onClick={() => setView(selectedAssistant ? 'detail' : 'list')}
              className="grid size-8 shrink-0 place-items-center rounded-md text-neutral-500 hover:bg-neutral-100 hover:text-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
              aria-label="返回"
            >
              <ArrowLeft size={18} />
            </button>
            <div className="min-w-0">
              <h2 className="truncate text-[24px] font-semibold text-neutral-950 dark:text-neutral-50">編輯套件</h2>
              <p className="mt-1 truncate text-[13px] text-neutral-500">
                {draft.built_in ? '內建套件範本' : '自訂套件'} · {draft.enabled === false ? '已停用' : '可用'}
              </p>
            </div>
          </div>
          <div className="flex shrink-0 flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void handleDelete()}
              disabled={saving}
              className="grid h-9 w-9 place-items-center rounded-md text-neutral-500 hover:bg-red-50 hover:text-red-600 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-red-950/30 dark:hover:text-red-300"
              title="刪除"
              aria-label="刪除套件"
            >
              <Trash2 size={15} />
            </button>
            <button
              type="button"
              onClick={() => void saveDraft().then((saved) => {
                if (saved) setView('detail')
              })}
              disabled={saving}
              className="flex h-9 items-center gap-2 rounded-md px-3 text-[13px] font-medium text-neutral-700 hover:bg-neutral-100 disabled:cursor-not-allowed disabled:opacity-40 dark:text-neutral-200 dark:hover:bg-neutral-800"
            >
              <Save size={15} />
              儲存
            </button>
            <button
              type="button"
              onClick={() => void handleStartChat()}
              disabled={saving || draft.enabled === false}
              className="flex h-9 items-center gap-2 rounded-md bg-neutral-950 px-3 text-[13px] font-medium text-white hover:bg-neutral-700 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-neutral-200"
            >
              <Play size={15} />
              開始聊天
            </button>
          </div>
        </div>

        <div className="space-y-6">
          <section className="space-y-5">
            <div className="grid gap-3 sm:grid-cols-[7rem_minmax(0,1fr)]">
              <label className="block">
                <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">標識</span>
                <input
                  type="text"
                  value={draft.icon ?? ''}
                  onChange={(event) => updateDraft('icon', event.target.value)}
                  className="h-10 w-full rounded-md border border-neutral-200 bg-white px-3 text-[13px] outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </label>
              <label className="block">
                <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">名稱</span>
                <input
                  type="text"
                  value={draft.name}
                  onChange={(event) => updateDraft('name', event.target.value)}
                  className="h-10 w-full rounded-md border border-neutral-200 bg-white px-3 text-[15px] font-medium outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </label>
            </div>
            <label className="block">
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">描述</span>
              <input
                type="text"
                value={draft.description ?? ''}
                onChange={(event) => updateDraft('description', event.target.value)}
                className="h-10 w-full rounded-md border border-neutral-200 bg-white px-3 text-[13px] outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
            <label className="block">
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">系統提示詞</span>
              <textarea
                value={draft.system_prompt ?? ''}
                onChange={(event) => updateDraft('system_prompt', event.target.value)}
                rows={9}
                className="custom-scrollbar w-full resize-none rounded-md border border-neutral-200 bg-white px-3 py-2.5 text-[13px] leading-relaxed text-neutral-900 outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-[12px] font-medium text-neutral-600 dark:text-neutral-300">MCP 伺服器</span>
                  <span className="text-[11px] text-neutral-400">{draftMcpIds.length} 已選</span>
                </div>
                <div className="custom-scrollbar max-h-56 space-y-1 overflow-y-auto rounded-md border border-neutral-200 p-2 dark:border-neutral-700">
                  {mcpServers.length === 0 ? (
                    <div className="px-1 py-2 text-[12px] text-neutral-400">未配置 MCP 伺服器（在「MCP」設定裡新增）</div>
                  ) : mcpServers.map((server) => (
                    <label key={server.id} className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1.5 text-[13px] hover:bg-neutral-50 dark:hover:bg-neutral-800">
                      <input
                        type="checkbox"
                        checked={draftMcpIds.includes(server.id)}
                        onChange={() => updateDraft('mcp_server_ids', toggleId(draftMcpIds, server.id))}
                        className="size-4 accent-neutral-900 dark:accent-neutral-100"
                      />
                      <span className="min-w-0 truncate text-neutral-700 dark:text-neutral-200">{server.name}</span>
                    </label>
                  ))}
                </div>
              </div>
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-[12px] font-medium text-neutral-600 dark:text-neutral-300">技能</span>
                  <span className="text-[11px] text-neutral-400">{draftSkillIds.length} 已選</span>
                </div>
                <div className="custom-scrollbar max-h-56 space-y-1 overflow-y-auto rounded-md border border-neutral-200 p-2 dark:border-neutral-700">
                  {skills.length === 0 ? (
                    <div className="px-1 py-2 text-[12px] text-neutral-400">沒有可用技能</div>
                  ) : skills.map((skill) => (
                    <label key={skill.id} className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1.5 text-[13px] hover:bg-neutral-50 dark:hover:bg-neutral-800">
                      <input
                        type="checkbox"
                        checked={draftSkillIds.includes(skill.id)}
                        onChange={() => updateDraft('skill_ids', toggleId(draftSkillIds, skill.id))}
                        className="size-4 accent-neutral-900 dark:accent-neutral-100"
                      />
                      <span className="min-w-0 truncate text-neutral-700 dark:text-neutral-200">{skill.name}</span>
                    </label>
                  ))}
                </div>
              </div>
            </div>
          </section>

          <section className="grid gap-4 lg:grid-cols-3">
            <section className="space-y-3 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200">執行設定</div>
              <label className="block">
                <span className="mb-1 block text-[11px] text-neutral-500 dark:text-neutral-400">模型供應商</span>
                <Select
                  value={draft.provider_id ?? ''}
                  onChange={(providerId) => {
                    const provider = providers.find((item) => item.id === providerId)
                    updateDraft('provider_id', providerId)
                    updateDraft('model', providerModels(provider)[0] ?? '')
                  }}
                  options={[
                    { value: '', label: '跟隨聊天預設' },
                    ...enabledProviders.map((provider) => ({
                      value: provider.id,
                      label: provider.name,
                    })),
                  ]}
                />
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] text-neutral-500 dark:text-neutral-400">模型</span>
                <Select
                  value={draft.model ?? ''}
                  onChange={(model) => updateDraft('model', model)}
                  options={
                    draft.provider_id
                      ? models.map((model) => ({ value: model, label: model }))
                      : [{ value: '', label: '跟隨聊天預設' }]
                  }
                />
              </label>
              <label className="flex items-center justify-between gap-3 rounded-md bg-neutral-50 px-2.5 py-2 text-[12px] text-neutral-700 dark:bg-neutral-800/70 dark:text-neutral-200">
                <span>啟用助手</span>
                <input
                  type="checkbox"
                  checked={draft.enabled !== false}
                  onChange={(event) => updateDraft('enabled', event.target.checked)}
                  className="size-4 accent-neutral-900 dark:accent-neutral-100"
                />
              </label>
            </section>

            <section className="space-y-3 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200">顏色</div>
              <div className="flex flex-wrap gap-1.5">
                {assistantColors.map((color) => (
                  <button
                    key={color}
                    type="button"
                    onClick={() => updateDraft('color', color)}
                    className={`size-6 rounded-full border ${
                      draft.color === color
                        ? 'border-neutral-900 ring-2 ring-neutral-300 dark:border-neutral-100 dark:ring-neutral-600'
                        : 'border-transparent'
                    }`}
                    style={{ backgroundColor: color }}
                    aria-label={`選擇顏色 ${color}`}
                  />
                ))}
              </div>
            </section>

            <section className="space-y-2 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200">當前配置</div>
              <div className="space-y-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                <div className="truncate">模型：{draft.model || '跟隨聊天預設'}</div>
                <div className="truncate">MCP：{draftMcpIds.length} 個</div>
                <div className="truncate">技能：{draftSkillIds.length} 個</div>
              </div>
              {providers.length === 0 && (
                <div className="rounded-md bg-amber-50 px-2 py-1.5 text-[11px] text-amber-700 dark:bg-amber-950/30 dark:text-amber-300">
                  未讀取到模型供應商
                </div>
              )}
            </section>
          </section>
        </div>
      </div>
    )
  }

  return (
    <div className="assistant-center-root flex h-full min-h-0 flex-col text-neutral-900 dark:text-neutral-100">
      {/* 頂欄：與聊天主區同底色、無分隔，可拖拽，右側避開視窗按鈕 */}
      <div
        className={`flex h-[52px] shrink-0 items-center gap-2 px-3 ${
          !usesNativeTitlebar ? 'chat-win-titlebar-safe' : ''
        }`}
        data-tauri-drag-region
      >
        <button
          type="button"
          onClick={onClose}
          className="flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-[13px] text-neutral-600 transition-colors hover:bg-black/[0.06] hover:text-neutral-900 dark:text-neutral-300 dark:hover:bg-white/[0.08] dark:hover:text-neutral-100"
          data-tauri-drag-region="false"
        >
          <ArrowLeft size={15} />
          返回聊天
        </button>
        <div className="h-full min-w-5 flex-1" data-tauri-drag-region />
      </div>

      {/* 內容區：直接坐在白底上，與聊天主區無縫 */}
      <main className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-6 py-6">
          <div className="mx-auto max-w-7xl space-y-4">
            <header className="flex min-w-0 items-center gap-3">
              <div className="flex min-w-0 shrink-0 items-center gap-2">
                <h1 className="truncate text-[24px] font-semibold tracking-normal text-neutral-950 dark:text-neutral-50">
                  專家套件
                </h1>
                <button
                  type="button"
                  onClick={() => void loadAssistants(selectedId)}
                  className="grid size-9 shrink-0 place-items-center rounded-md text-neutral-500 hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                  aria-label="重新整理套件"
                  title="重新整理"
                >
                  <RefreshCw size={16} />
                </button>
              </div>
              <div className="assistant-center-toolbar ml-auto flex min-w-0 flex-1 items-center justify-end gap-2">
                <div className="assistant-center-search relative min-w-[180px] flex-1 sm:max-w-[360px]">
                  <Search
                    size={16}
                    className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-neutral-400"
                  />
                  <input
                    type="text"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="搜尋套件..."
                    className="h-9 w-full rounded-md border border-neutral-200 bg-white pl-9 pr-3 text-[13px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                  />
                </div>
                {onStartBuilder && (
                  <button
                    type="button"
                    onClick={() => onStartBuilder()}
                    className="flex h-9 shrink-0 items-center justify-center gap-2 rounded-md border border-neutral-200 bg-white px-3 text-[13px] font-medium text-neutral-700 hover:bg-neutral-100 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200 dark:hover:bg-neutral-800"
                    title="通過對話搭建一個新專家"
                  >
                    <Sparkles size={16} />
                    AI 建立
                  </button>
                )}
                <button
                  type="button"
                  onClick={handleCreate}
                  className="flex h-9 shrink-0 items-center justify-center gap-2 rounded-md bg-neutral-950 px-3 text-[13px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-neutral-200"
                >
                  <Plus size={16} />
                  建立
                </button>
              </div>
            </header>

            {error && (
              <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
                {error}
              </div>
            )}

            {view === 'list' && renderList()}
            {view === 'detail' && renderDetail()}
            {view === 'edit' && renderEdit()}
          </div>
        </main>
    </div>
  )
}
