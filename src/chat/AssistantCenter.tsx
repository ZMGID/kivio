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
  Trash2,
  Wrench,
} from 'lucide-react'
import { api, type ModelProvider } from '../api/tauri'
import { isProviderEnabled } from '../settings/utils'
import { chatApi } from './api'
import type {
  AssistantDataConnector,
  AssistantKnowledgeSkill,
  AssistantQuickCommand,
  AssistantToolPreset,
  ChatAssistant,
  SkillMeta,
} from './types'

interface AssistantCenterProps {
  skills: SkillMeta[]
  currentAssistantId?: string | null
  onStartAssistantChat: (assistant: ChatAssistant) => void
  onApplyAssistant?: (assistantId: string | null) => void
  onClose: () => void
}

type AssistantDraft = ChatAssistant
type CenterView = 'list' | 'detail' | 'edit'
type SuiteTab = 'plaza' | 'installed' | 'mine'

const toolPresetOptions: Array<{ value: AssistantToolPreset; label: string }> = [
  { value: 'inherit', label: '跟随聊天设置' },
  { value: 'none', label: '不使用工具' },
  { value: 'skills', label: '仅 Skill 工具' },
  { value: 'all', label: '全部可用工具' },
]

const assistantColors = ['#6A8FBD', '#C56646', '#4F9D7A', '#8A6FBD', '#B7791F', '#5E8C6A']

function nowSeconds() {
  return Math.floor(Date.now() / 1000)
}

function listFromMaybe<T>(snake?: T[], camel?: T[]): T[] {
  return Array.isArray(snake) ? snake : Array.isArray(camel) ? camel : []
}

function assistantQuickCommands(assistant?: ChatAssistant | null): AssistantQuickCommand[] {
  return listFromMaybe(assistant?.quick_commands, assistant?.quickCommands)
}

function assistantDataConnectors(assistant?: ChatAssistant | null): AssistantDataConnector[] {
  return listFromMaybe(assistant?.data_connectors, assistant?.dataConnectors)
}

function assistantKnowledgeSkills(assistant?: ChatAssistant | null): AssistantKnowledgeSkill[] {
  return listFromMaybe(assistant?.knowledge_skills, assistant?.knowledgeSkills)
}

function normalizeStringList(values?: string[], limit = 8): string[] {
  const out: string[] = []
  for (const value of values ?? []) {
    const item = value.trim()
    if (!item || out.includes(item)) continue
    out.push(item)
    if (out.length >= limit) break
  }
  return out
}

function normalizeSlash(value: string, fallback: string): string {
  const source = (value.trim() || fallback.trim()).replace(/\s+/g, '')
  if (!source) return ''
  return source.startsWith('/') ? source : `/${source}`
}

function normalizeCommand(command: AssistantQuickCommand, index: number): AssistantQuickCommand | null {
  const name = command.name.trim()
  const slash = normalizeSlash(command.slash, name)
  if (!name || !slash) return null
  return {
    id: command.id?.trim() || `cmd_${index}`,
    name,
    slash,
    description: command.description?.trim() ?? '',
    placeholder: command.placeholder?.trim() ?? '',
    prompt: command.prompt?.trim() ?? '',
    starter_text: (command.starter_text ?? command.starterText ?? '').trim(),
    requires_suite_enabled: command.requires_suite_enabled ?? command.requiresSuiteEnabled ?? true,
    enabled: command.enabled ?? true,
  }
}

function normalizeConnector(connector: AssistantDataConnector, index: number): AssistantDataConnector | null {
  const name = connector.name.trim()
  if (!name) return null
  return {
    id: connector.id?.trim() || `conn_${index}`,
    name,
    kind: connector.kind || 'builtin_tool',
    description: connector.description?.trim() ?? '',
    tool_ids: normalizeStringList(connector.tool_ids ?? connector.toolIds, 12),
    server_id: connector.server_id ?? connector.serverId ?? null,
    required: connector.required ?? false,
    enabled: connector.enabled ?? true,
    configured: connector.configured ?? true,
  }
}

function normalizeKnowledgeSkill(skill: AssistantKnowledgeSkill, index: number): AssistantKnowledgeSkill | null {
  const name = skill.name.trim()
  if (!name) return null
  return {
    id: skill.id?.trim() || `ks_${index}`,
    name,
    description: skill.description?.trim() ?? '',
    trigger_phrases: normalizeStringList(skill.trigger_phrases ?? skill.triggerPhrases, 16),
    skill_id: skill.skill_id ?? skill.skillId ?? null,
    prompt: skill.prompt?.trim() ?? '',
    recommended_tools: normalizeStringList(skill.recommended_tools ?? skill.recommendedTools, 12),
    requires_connectors: normalizeStringList(skill.requires_connectors ?? skill.requiresConnectors, 12),
    enabled: skill.enabled ?? true,
  }
}

function normalizeAssistantForDraft(assistant: ChatAssistant): AssistantDraft {
  return {
    ...assistant,
    description: assistant.description ?? '',
    icon: assistant.icon ?? 'bot',
    color: assistant.color ?? '#6A8FBD',
    source: assistant.source ?? (assistant.built_in ?? assistant.builtIn ? 'builtin' : 'user'),
    author: assistant.author ?? (assistant.built_in ?? assistant.builtIn ? 'Kivio' : ''),
    version: assistant.version ?? '1.0.0',
    category: assistant.category ?? '',
    tags: assistant.tags ?? [],
    system_prompt: assistant.system_prompt ?? assistant.systemPrompt ?? '',
    provider_id: assistant.provider_id ?? assistant.providerId ?? '',
    model: assistant.model ?? '',
    skill_id: assistant.skill_id ?? assistant.skillId ?? null,
    tool_preset: assistant.tool_preset ?? assistant.toolPreset ?? 'inherit',
    conversation_starters: assistant.conversation_starters ?? assistant.conversationStarters ?? [],
    greeting: assistant.greeting ?? '',
    quick_commands: assistantQuickCommands(assistant),
    data_connectors: assistantDataConnectors(assistant),
    knowledge_skills: assistantKnowledgeSkills(assistant),
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
    name: '新套件',
    description: '',
    icon: 'bot',
    color: '#6A8FBD',
    source: 'user',
    author: '',
    version: '1.0.0',
    category: '',
    tags: [],
    system_prompt: '',
    provider_id: '',
    model: '',
    skill_id: null,
    tool_preset: 'inherit',
    conversation_starters: [],
    greeting: '',
    quick_commands: [],
    data_connectors: [],
    knowledge_skills: [],
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
    author: draft.author?.trim() ?? '',
    version: draft.version?.trim() || '1.0.0',
    category: draft.category?.trim() ?? '',
    tags: normalizeStringList(draft.tags, 8),
    system_prompt: (draft.system_prompt ?? draft.systemPrompt ?? '').trim(),
    provider_id: (draft.provider_id ?? draft.providerId ?? '').trim(),
    model: draft.provider_id ? (draft.model ?? '').trim() : '',
    skill_id: draft.skill_id || null,
    tool_preset: draft.tool_preset ?? 'inherit',
    conversation_starters: (draft.conversation_starters ?? [])
      .map((starter) => starter.trim())
      .filter(Boolean)
      .slice(0, 6),
    greeting: draft.greeting?.trim() ?? '',
    quick_commands: assistantQuickCommands(draft)
      .map(normalizeCommand)
      .filter((command): command is AssistantQuickCommand => Boolean(command))
      .slice(0, 12),
    data_connectors: assistantDataConnectors(draft)
      .map(normalizeConnector)
      .filter((connector): connector is AssistantDataConnector => Boolean(connector))
      .slice(0, 12),
    knowledge_skills: assistantKnowledgeSkills(draft)
      .map(normalizeKnowledgeSkill)
      .filter((skill): skill is AssistantKnowledgeSkill => Boolean(skill))
      .slice(0, 12),
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
  const text = [
    assistant.name,
    assistant.description,
    assistant.author,
    assistant.category,
    ...(assistant.tags ?? []),
    assistant.system_prompt ?? assistant.systemPrompt,
    ...(assistant.conversation_starters ?? assistant.conversationStarters ?? []),
    ...assistantQuickCommands(assistant).flatMap((command) => [command.name, command.slash, command.description]),
    ...assistantDataConnectors(assistant).flatMap((connector) => [connector.name, connector.description]),
    ...assistantKnowledgeSkills(assistant).flatMap((skill) => [skill.name, skill.description]),
  ].join('\n').toLowerCase()
  return text.includes(query)
}

function providerModels(provider?: ModelProvider): string[] {
  if (!provider) return []
  return provider.enabledModels.length > 0 ? provider.enabledModels : provider.availableModels
}

function suiteStats(assistant: ChatAssistant) {
  return {
    commands: assistantQuickCommands(assistant).filter((command) => command.enabled !== false).length,
    connectors: assistantDataConnectors(assistant).filter((connector) => connector.enabled !== false).length,
    skills: assistantKnowledgeSkills(assistant).filter((skill) => skill.enabled !== false).length,
  }
}

function linesToCommands(value: string): AssistantQuickCommand[] {
  return value.split('\n').map((line, index) => {
    const [slash = '', name = '', description = '', prompt = ''] = line.split('|')
    return {
      id: `cmd_${index}`,
      slash: slash.trim(),
      name: name.trim() || slash.replace('/', '').trim(),
      description: description.trim(),
      prompt: prompt.trim(),
      enabled: true,
      requires_suite_enabled: true,
    }
  }).filter((command) => command.name.trim() || command.slash.trim())
}

function commandsToLines(commands: AssistantQuickCommand[]): string {
  return commands.map((command) => [
    command.slash,
    command.name,
    command.description ?? '',
    command.prompt ?? '',
  ].join(' | ')).join('\n')
}

function linesToConnectors(value: string): AssistantDataConnector[] {
  return value.split('\n').map((line, index) => {
    const [name = '', kind = '', tools = '', description = ''] = line.split('|')
    return {
      id: `conn_${index}`,
      name: name.trim(),
      kind: kind.trim() || 'builtin_tool',
      tool_ids: tools.split(',').map((tool) => tool.trim()).filter(Boolean),
      description: description.trim(),
      enabled: true,
      configured: true,
    }
  }).filter((connector) => connector.name.trim())
}

function connectorsToLines(connectors: AssistantDataConnector[]): string {
  return connectors.map((connector) => [
    connector.name,
    connector.kind ?? 'builtin_tool',
    (connector.tool_ids ?? connector.toolIds ?? []).join(', '),
    connector.description ?? '',
  ].join(' | ')).join('\n')
}

function linesToKnowledgeSkills(value: string): AssistantKnowledgeSkill[] {
  return value.split('\n').map((line, index) => {
    const [name = '', triggers = '', description = '', prompt = ''] = line.split('|')
    return {
      id: `ks_${index}`,
      name: name.trim(),
      trigger_phrases: triggers.split(',').map((trigger) => trigger.trim()).filter(Boolean),
      description: description.trim(),
      prompt: prompt.trim(),
      enabled: true,
    }
  }).filter((skill) => skill.name.trim())
}

function knowledgeSkillsToLines(skills: AssistantKnowledgeSkill[]): string {
  return skills.map((skill) => [
    skill.name,
    (skill.trigger_phrases ?? skill.triggerPhrases ?? []).join(', '),
    skill.description ?? '',
    skill.prompt ?? '',
  ].join(' | ')).join('\n')
}

export function AssistantCenter({
  skills,
  currentAssistantId,
  onStartAssistantChat,
  onApplyAssistant,
  onClose,
}: AssistantCenterProps) {
  const [assistants, setAssistants] = useState<ChatAssistant[]>([])
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(currentAssistantId ?? null)
  const [draft, setDraft] = useState<AssistantDraft | null>(null)
  const [query, setQuery] = useState('')
  const [view, setView] = useState<CenterView>('list')
  const [tab, setTab] = useState<SuiteTab>('plaza')
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
      setError(typeof err === 'string' ? err : (err as Error).message || '套件加载失败')
    } finally {
      setLoading(false)
    }
  }, [currentAssistantId])

  const loadProviders = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      setProviders(settings.providers || [])
    } catch {
      setProviders([])
    }
  }, [])

  useEffect(() => {
    void loadAssistants(currentAssistantId)
    void loadProviders()
  }, [currentAssistantId, loadAssistants, loadProviders])

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
  const selectedSkill = skills.find((skill) => skill.id === (draft?.skill_id ?? draft?.skillId))
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
      setError('套件名称不能为空')
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
      setError(typeof err === 'string' ? err : (err as Error).message || '套件保存失败')
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
      setError(typeof err === 'string' ? err : (err as Error).message || '复制失败')
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
    if (!window.confirm(`确定删除套件「${draft.name}」？已有对话会保留当时的套件快照。`)) return
    setSaving(true)
    setError('')
    try {
      await chatApi.deleteAssistant(draft.id)
      await loadAssistants(null)
      setView('list')
    } catch (err) {
      setError(typeof err === 'string' ? err : (err as Error).message || '删除失败')
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
            ['plaza', '套件广场', builtInCount],
            ['installed', '已安装', installedCount],
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
        <div className="grid min-h-[220px] place-items-center text-[13px] text-neutral-400">加载中...</div>
      ) : filteredAssistants.length === 0 ? (
        <div className="grid min-h-[220px] place-items-center rounded-md border border-dashed border-neutral-200 text-[13px] text-neutral-400 dark:border-neutral-800">
          没有匹配的套件
        </div>
      ) : (
        <div className="grid gap-3 xl:grid-cols-2">
          {filteredAssistants.map((assistant) => {
            const stats = suiteStats(assistant)
            const builtIn = assistant.built_in ?? assistant.builtIn ?? false
            return (
              <article
                key={assistant.id}
                className="min-w-0 rounded-md bg-neutral-50 p-3.5 transition-colors hover:bg-neutral-100 dark:bg-neutral-900/60 dark:hover:bg-neutral-900"
              >
                <div className="flex min-w-0 gap-3">
                  <button
                    type="button"
                    onClick={() => openDetail(assistant)}
                    className="grid size-11 shrink-0 place-items-center rounded-md border border-neutral-200 bg-white text-[17px] font-semibold text-neutral-600 hover:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-300"
                    style={{ color: assistant.color || '#6A8FBD' }}
                    aria-label={`打开 ${assistant.name}`}
                  >
                    {assistant.name.trim().slice(0, 1) || '套'}
                  </button>
                  <button
                    type="button"
                    onClick={() => openDetail(assistant)}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-[15px] font-semibold text-neutral-950 dark:text-neutral-50">
                        {assistant.name}
                      </span>
                      {builtIn && (
                        <span className="shrink-0 rounded bg-emerald-100 px-1.5 py-0.5 text-[11px] text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">
                          内置
                        </span>
                      )}
                    </div>
                    <div className="mt-0.5 truncate text-[12px] font-medium text-neutral-500">
                      @{assistant.author || (builtIn ? 'Kivio' : 'Local')}
                    </div>
                    <p className="mt-2 line-clamp-2 text-[12px] leading-relaxed text-neutral-600 dark:text-neutral-400">
                      {assistant.description || '未设置描述'}
                    </p>
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleStartChat(assistant)}
                    disabled={assistant.enabled === false}
                    className="grid size-8 shrink-0 place-items-center rounded-md text-neutral-900 hover:bg-white disabled:cursor-not-allowed disabled:opacity-40 dark:text-neutral-100 dark:hover:bg-neutral-800"
                    aria-label={`使用 ${assistant.name} 开始聊天`}
                    title="开始聊天"
                  >
                    <Plus size={20} />
                  </button>
                </div>
                <div className="mt-4 flex items-center justify-between gap-3 text-[12px] text-neutral-500 dark:text-neutral-400">
                  <div className="flex min-w-0 flex-wrap gap-x-4 gap-y-1">
                    <span>{stats.skills} 个技能</span>
                    <span>{stats.connectors} 个数据连接</span>
                    <span>{stats.commands} 个快捷命令</span>
                  </div>
                  <span className="shrink-0">v{assistant.version || '1.0.0'}</span>
                </div>
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
    const commands = assistantQuickCommands(assistant)
    const connectors = assistantDataConnectors(assistant)
    const knowledge = assistantKnowledgeSkills(assistant)
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
              {assistant.name.trim().slice(0, 1) || '套'}
            </div>
            <div className="min-w-0">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <h2 className="truncate text-[28px] font-semibold tracking-normal text-neutral-950 dark:text-neutral-50">
                  {assistant.name}
                </h2>
                <span className="rounded bg-emerald-100 px-2 py-1 text-[12px] font-medium text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">
                  v{assistant.version || '1.0.0'}
                </span>
              </div>
              <div className="mt-1 text-[13px] font-medium text-neutral-500">@{assistant.author || 'Kivio'}</div>
              <p className="mt-6 max-w-5xl text-[16px] leading-8 text-neutral-700 dark:text-neutral-300">
                {assistant.description || '这个套件还没有描述。'}
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
              编辑
            </button>
            <button
              type="button"
              onClick={() => void handleDuplicate(assistant)}
              className="grid h-9 w-9 place-items-center rounded-md bg-neutral-100 text-neutral-700 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
              title="复制套件"
              aria-label="复制套件"
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
                应用到当前对话
              </button>
            )}
            <button
              type="button"
              onClick={() => void handleStartChat(assistant)}
              disabled={assistant.enabled === false}
              className="flex h-9 items-center gap-2 rounded-md bg-neutral-950 px-3 text-[13px] font-medium text-white hover:bg-neutral-700 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-neutral-200"
            >
              <Play size={15} />
              开始聊天
            </button>
          </div>
        </div>

        <section className="space-y-3">
          <h3 className="text-[17px] font-semibold text-neutral-950 dark:text-neutral-50">
            快捷命令 <span className="text-neutral-400">({commands.length})</span>
          </h3>
          <div className="overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
            {commands.length === 0 ? (
              <div className="px-4 py-5 text-[13px] text-neutral-400">暂无快捷命令</div>
            ) : commands.map((command) => (
              <button
                key={command.id}
                type="button"
                onClick={() => void handleStartChat(assistant)}
                disabled={assistant.enabled === false || command.enabled === false}
                className="flex w-full min-w-0 items-center gap-4 border-b border-neutral-200 px-4 py-3 text-left last:border-b-0 hover:bg-neutral-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-neutral-800 dark:hover:bg-neutral-900/70"
              >
                <span className="shrink-0 rounded bg-emerald-50 px-2.5 py-1 text-[14px] font-semibold text-emerald-600 dark:bg-emerald-950/40 dark:text-emerald-300">
                  {command.slash}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium text-neutral-800 dark:text-neutral-100">
                    {command.name}
                  </span>
                  <span className="mt-0.5 block truncate text-[12px] text-neutral-500">
                    {command.description || command.placeholder || '启动这个套件的专用任务'}
                  </span>
                </span>
                <Play size={15} className="shrink-0 text-neutral-300" />
              </button>
            ))}
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-[17px] font-semibold text-neutral-950 dark:text-neutral-50">
            数据连接 <span className="text-neutral-400">({connectors.length})</span>
          </h3>
          <div className="overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
            {connectors.length === 0 ? (
              <div className="px-4 py-5 text-[13px] text-neutral-400">暂无数据连接</div>
            ) : connectors.map((connector) => (
              <div
                key={connector.id}
                className="flex min-w-0 items-center gap-4 border-b border-neutral-200 px-4 py-4 last:border-b-0 dark:border-neutral-800"
              >
                <Wrench size={16} className="shrink-0 text-neutral-400" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">
                    {connector.name}
                  </div>
                  <div className="mt-1 line-clamp-2 text-[12px] text-neutral-500">
                    {connector.description || connector.kind || '数据连接'}
                  </div>
                </div>
                <span className={`shrink-0 rounded-full px-2 py-1 text-[11px] ${
                  connector.enabled === false || connector.configured === false
                    ? 'bg-neutral-100 text-neutral-400 dark:bg-neutral-800'
                    : 'bg-emerald-50 text-emerald-600 dark:bg-emerald-950/40 dark:text-emerald-300'
                }`}>
                  {connector.enabled === false || connector.configured === false ? '未启用' : '可用'}
                </span>
              </div>
            ))}
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-[17px] font-semibold text-neutral-950 dark:text-neutral-50">
            知识技能 <span className="text-neutral-400">({knowledge.length})</span>
          </h3>
          <div className="overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
            {knowledge.length === 0 ? (
              <div className="px-4 py-5 text-[13px] text-neutral-400">暂无知识技能</div>
            ) : knowledge.map((skill) => (
              <div
                key={skill.id}
                className="border-b border-neutral-200 px-4 py-4 last:border-b-0 dark:border-neutral-800"
              >
                <div className="flex min-w-0 items-center gap-3">
                  <BookOpen size={16} className="shrink-0 text-neutral-400" />
                  <div className="min-w-0 flex-1 truncate text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">
                    {skill.name}
                  </div>
                  {skill.skill_id && (
                    <span className="shrink-0 rounded bg-neutral-100 px-2 py-1 text-[11px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                      Skill
                    </span>
                  )}
                </div>
                <p className="mt-2 line-clamp-3 text-[13px] leading-relaxed text-neutral-600 dark:text-neutral-400">
                  {skill.description || skill.prompt || '未设置说明'}
                </p>
              </div>
            ))}
          </div>
        </section>
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
              <h2 className="truncate text-[24px] font-semibold text-neutral-950 dark:text-neutral-50">编辑套件</h2>
              <p className="mt-1 truncate text-[13px] text-neutral-500">
                {draft.built_in ? '内置套件模板' : '自定义套件'} · {draft.enabled === false ? '已停用' : '可用'}
              </p>
            </div>
          </div>
          <div className="flex shrink-0 flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void handleDelete()}
              disabled={saving}
              className="grid h-9 w-9 place-items-center rounded-md text-neutral-500 hover:bg-red-50 hover:text-red-600 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-red-950/30 dark:hover:text-red-300"
              title="删除"
              aria-label="删除套件"
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
              保存
            </button>
            <button
              type="button"
              onClick={() => void handleStartChat()}
              disabled={saving || draft.enabled === false}
              className="flex h-9 items-center gap-2 rounded-md bg-neutral-950 px-3 text-[13px] font-medium text-white hover:bg-neutral-700 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-neutral-200"
            >
              <Play size={15} />
              开始聊天
            </button>
          </div>
        </div>

        <div className="space-y-6">
          <section className="space-y-5">
            <div className="grid gap-3 sm:grid-cols-[7rem_minmax(0,1fr)]">
              <label className="block">
                <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">标识</span>
                <input
                  type="text"
                  value={draft.icon ?? ''}
                  onChange={(event) => updateDraft('icon', event.target.value)}
                  className="h-10 w-full rounded-md border border-neutral-200 bg-white px-3 text-[13px] outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </label>
              <label className="block">
                <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">名称</span>
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
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">系统提示词</span>
              <textarea
                value={draft.system_prompt ?? ''}
                onChange={(event) => updateDraft('system_prompt', event.target.value)}
                rows={9}
                className="custom-scrollbar w-full resize-none rounded-md border border-neutral-200 bg-white px-3 py-2.5 text-[13px] leading-relaxed text-neutral-900 outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
            <div className="grid gap-4 md:grid-cols-2">
              <label className="block">
                <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">开场白</span>
                <input
                  type="text"
                  value={draft.greeting ?? ''}
                  onChange={(event) => updateDraft('greeting', event.target.value)}
                  className="h-10 w-full rounded-md border border-neutral-200 bg-white px-3 text-[13px] outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </label>
              <label className="block">
                <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">标签</span>
                <input
                  type="text"
                  value={(draft.tags ?? []).join(', ')}
                  onChange={(event) => updateDraft('tags', event.target.value.split(',').map((tag) => tag.trim()))}
                  className="h-10 w-full rounded-md border border-neutral-200 bg-white px-3 text-[13px] outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </label>
            </div>
            <label className="block">
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">开场问题</span>
              <textarea
                value={(draft.conversation_starters ?? []).join('\n')}
                onChange={(event) => updateDraft('conversation_starters', event.target.value.split('\n').slice(0, 6))}
                rows={4}
                className="custom-scrollbar w-full resize-none rounded-md border border-neutral-200 bg-white px-3 py-2.5 text-[13px] leading-relaxed outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
            <label className="block">
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">快捷命令</span>
              <textarea
                value={commandsToLines(assistantQuickCommands(draft))}
                onChange={(event) => updateDraft('quick_commands', linesToCommands(event.target.value))}
                rows={5}
                className="custom-scrollbar w-full resize-none rounded-md border border-neutral-200 bg-white px-3 py-2.5 text-[13px] leading-relaxed outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
            <label className="block">
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">知识技能</span>
              <textarea
                value={knowledgeSkillsToLines(assistantKnowledgeSkills(draft))}
                onChange={(event) => updateDraft('knowledge_skills', linesToKnowledgeSkills(event.target.value))}
                rows={5}
                className="custom-scrollbar w-full resize-none rounded-md border border-neutral-200 bg-white px-3 py-2.5 text-[13px] leading-relaxed outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
            <label className="block">
              <span className="mb-1.5 block text-[12px] font-medium text-neutral-600 dark:text-neutral-300">数据连接</span>
              <textarea
                value={connectorsToLines(assistantDataConnectors(draft))}
                onChange={(event) => updateDraft('data_connectors', linesToConnectors(event.target.value))}
                rows={4}
                className="custom-scrollbar w-full resize-none rounded-md border border-neutral-200 bg-white px-3 py-2.5 text-[13px] leading-relaxed outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </label>
          </section>

          <section className="grid gap-4 lg:grid-cols-3">
            <section className="space-y-3 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200">运行设置</div>
              <label className="block">
                <span className="mb-1 block text-[11px] text-neutral-500 dark:text-neutral-400">模型供应商</span>
                <select
                  value={draft.provider_id ?? ''}
                  onChange={(event) => {
                    const providerId = event.target.value
                    const provider = providers.find((item) => item.id === providerId)
                    updateDraft('provider_id', providerId)
                    updateDraft('model', providerModels(provider)[0] ?? '')
                  }}
                  className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2 text-[12px] outline-none dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-100"
                >
                  <option value="">跟随聊天默认</option>
                  {enabledProviders.map((provider) => (
                    <option key={provider.id} value={provider.id}>{provider.name}</option>
                  ))}
                </select>
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] text-neutral-500 dark:text-neutral-400">模型</span>
                <select
                  value={draft.model ?? ''}
                  disabled={!draft.provider_id}
                  onChange={(event) => updateDraft('model', event.target.value)}
                  className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2 text-[12px] outline-none disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-100"
                >
                  {!draft.provider_id && <option value="">跟随聊天默认</option>}
                  {models.map((model) => (
                    <option key={model} value={model}>{model}</option>
                  ))}
                </select>
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] text-neutral-500 dark:text-neutral-400">默认 Skill</span>
                <select
                  value={draft.skill_id ?? ''}
                  onChange={(event) => updateDraft('skill_id', event.target.value || null)}
                  className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2 text-[12px] outline-none dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-100"
                >
                  <option value="">不绑定</option>
                  {skills.map((skill) => (
                    <option key={skill.id} value={skill.id}>{skill.name}</option>
                  ))}
                </select>
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] text-neutral-500 dark:text-neutral-400">工具策略</span>
                <select
                  value={draft.tool_preset ?? 'inherit'}
                  onChange={(event) => updateDraft('tool_preset', event.target.value)}
                  className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2 text-[12px] outline-none dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-100"
                >
                  {toolPresetOptions.map((option) => (
                    <option key={option.value} value={option.value}>{option.label}</option>
                  ))}
                </select>
              </label>
              <label className="flex items-center justify-between gap-3 rounded-md bg-neutral-50 px-2.5 py-2 text-[12px] text-neutral-700 dark:bg-neutral-800/70 dark:text-neutral-200">
                <span>启用套件</span>
                <input
                  type="checkbox"
                  checked={draft.enabled !== false}
                  onChange={(event) => updateDraft('enabled', event.target.checked)}
                  className="size-4 accent-neutral-900 dark:accent-neutral-100"
                />
              </label>
            </section>

            <section className="space-y-3 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200">颜色</div>
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
                    aria-label={`选择颜色 ${color}`}
                  />
                ))}
              </div>
            </section>

            <section className="space-y-2 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-[12px] font-semibold text-neutral-700 dark:text-neutral-200">当前配置</div>
              <div className="space-y-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                <div className="truncate">模型：{draft.model || '跟随聊天默认'}</div>
                <div className="truncate">Skill：{selectedSkill?.name || '不绑定'}</div>
                <div className="truncate">工具：{toolPresetOptions.find((item) => item.value === draft.tool_preset)?.label ?? '跟随聊天设置'}</div>
                <div className="truncate">命令：{assistantQuickCommands(draft).length} 个</div>
              </div>
              {providers.length === 0 && (
                <div className="rounded-md bg-amber-50 px-2 py-1.5 text-[11px] text-amber-700 dark:bg-amber-950/30 dark:text-amber-300">
                  未读取到模型供应商
                </div>
              )}
            </section>
          </section>
        </div>
      </div>
    )
  }

  return (
    <div className="assistant-center-root h-full min-h-0 text-neutral-900 dark:text-neutral-100">
      <main className="custom-scrollbar h-full min-h-0 overflow-y-auto px-6 py-6">
        <div className="mx-auto max-w-7xl space-y-4">
          <header className="assistant-center-header flex min-w-0 items-center gap-3">
            <div className="flex min-w-0 shrink-0 items-center gap-2">
              <button
                type="button"
                onClick={onClose}
                className="grid size-9 shrink-0 place-items-center rounded-md text-neutral-500 hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                aria-label="返回聊天"
                title="返回聊天"
              >
                <ArrowLeft size={16} />
              </button>
              <h1 className="truncate text-[24px] font-semibold tracking-normal text-neutral-950 dark:text-neutral-50">
                专家套件
              </h1>
              <button
                type="button"
                onClick={() => void loadAssistants(selectedId)}
                className="grid size-9 shrink-0 place-items-center rounded-md text-neutral-500 hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                aria-label="刷新套件"
                title="刷新"
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
                  placeholder="搜索套件..."
                  className="h-9 w-full rounded-md border border-neutral-200 bg-white pl-9 pr-3 text-[13px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </div>
              <button
                type="button"
                onClick={handleCreate}
                className="flex h-9 shrink-0 items-center justify-center gap-2 rounded-md bg-neutral-950 px-3 text-[13px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-neutral-200"
              >
                <Plus size={16} />
                创建
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
