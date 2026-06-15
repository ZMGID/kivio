// Chat API 调用封装
import { invoke } from '@tauri-apps/api/core'
import { estimateTokens } from '../utils/tokens'
import type {
  ChatAssistant,
  ChatAssistantSnapshot,
  ChatProject,
  Conversation,
  ConversationContextState,
  ConversationListItem,
  AgentPlanMode,
  PendingAttachment,
} from './types'

const isTauriRuntime = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

const mockStorageKey = 'kivio-chat-dev-conversations'
const mockProjectsStorageKey = 'kivio-chat-dev-projects'
const mockAssistantsStorageKey = 'kivio-chat-dev-assistants'

const nowSeconds = () => Math.floor(Date.now() / 1000)

function loadMockConversations(): Conversation[] {
  try {
    const raw = window.localStorage.getItem(mockStorageKey)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}

function saveMockConversations(conversations: Conversation[]) {
  window.localStorage.setItem(mockStorageKey, JSON.stringify(conversations))
}

function loadMockProjects(): ChatProject[] {
  try {
    const raw = window.localStorage.getItem(mockProjectsStorageKey)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}

function saveMockProjects(projects: ChatProject[]) {
  window.localStorage.setItem(mockProjectsStorageKey, JSON.stringify(projects))
}

function loadMockAssistants(): ChatAssistant[] {
  try {
    const raw = window.localStorage.getItem(mockAssistantsStorageKey)
    const parsed = raw ? JSON.parse(raw) : []
    const assistants = Array.isArray(parsed) ? parsed : []
    assistants.sort((a, b) => b.updated_at - a.updated_at || a.name.localeCompare(b.name, 'zh-CN'))
    return assistants
  } catch {
    return []
  }
}

function saveMockAssistants(assistants: ChatAssistant[]) {
  window.localStorage.setItem(mockAssistantsStorageKey, JSON.stringify(assistants))
}

function normalizeAssistant(assistant: ChatAssistant): ChatAssistant {
  const now = nowSeconds()
  return {
    ...assistant,
    name: assistant.name.trim(),
    description: assistant.description?.trim() ?? '',
    icon: assistant.icon?.trim() ?? '',
    color: assistant.color?.trim() ?? '',
    source: assistant.source ?? (assistant.built_in ?? assistant.builtIn ? 'builtin' : 'user'),
    system_prompt: (assistant.system_prompt ?? assistant.systemPrompt ?? '').trim(),
    provider_id: (assistant.provider_id ?? assistant.providerId ?? '').trim(),
    model: (assistant.model ?? '').trim(),
    mcp_server_ids: assistant.mcp_server_ids ?? assistant.mcpServerIds ?? [],
    skill_ids: assistant.skill_ids ?? assistant.skillIds ?? [],
    enabled: assistant.enabled ?? true,
    installed: assistant.installed ?? true,
    archived: assistant.archived ?? false,
    built_in: assistant.built_in ?? assistant.builtIn ?? false,
    created_at: assistant.created_at ?? assistant.createdAt ?? now,
    updated_at: now,
  }
}

function assistantSnapshot(assistant: ChatAssistant): ChatAssistantSnapshot {
  return {
    id: assistant.id,
    name: assistant.name,
    description: assistant.description ?? '',
    source: assistant.source,
    system_prompt: assistant.system_prompt ?? assistant.systemPrompt ?? '',
    provider_id: assistant.provider_id ?? assistant.providerId ?? '',
    model: assistant.model ?? '',
    mcp_server_ids: assistant.mcp_server_ids ?? assistant.mcpServerIds ?? [],
    skill_ids: assistant.skill_ids ?? assistant.skillIds ?? [],
  }
}

function normalizeProjectName(name: string): string {
  const trimmed = name.trim()
  if (!trimmed) throw new Error('项目名称不能为空')
  if ([...trimmed].length > 80) throw new Error('项目名称不能超过 80 个字符')
  return trimmed
}

function loadMockProjectsWithLegacyFolders(): ChatProject[] {
  const projects = loadMockProjects()
  const now = nowSeconds()
  let changed = false
  for (const folder of loadMockConversations()
    .map((conversation) => conversation.folder?.trim())
    .filter((folder): folder is string => Boolean(folder))) {
    if (projects.some((project) => project.name === folder)) continue
    projects.push({
      id: `proj_dev_${crypto.randomUUID()}`,
      name: folder,
      root_path: null,
      created_at: now,
      updated_at: now,
    })
    changed = true
  }
  projects.sort((a, b) => b.updated_at - a.updated_at || a.name.localeCompare(b.name, 'zh-CN'))
  if (changed) saveMockProjects(projects)
  return projects
}

function toListItem(conversation: Conversation): ConversationListItem {
  const preview = [...conversation.messages]
    .reverse()
    .find((message) => message.role === 'user' || message.role === 'assistant')
    ?.content.trim() ?? ''
  return {
    id: conversation.id,
    title: conversation.title,
    preview: preview.length > 100 ? `${preview.slice(0, 100)}...` : preview,
    provider_id: conversation.provider_id,
    model: conversation.model,
    message_count: conversation.messages.length,
    created_at: conversation.created_at,
    updated_at: conversation.updated_at,
    pinned: conversation.pinned,
    folder: conversation.folder,
    project_id: conversation.project_id ?? conversation.projectId ?? null,
    projectId: conversation.project_id ?? conversation.projectId ?? null,
    assistant_id: conversation.assistant_id ?? conversation.assistantId ?? null,
    assistant_name:
      conversation.assistant_snapshot?.name
      ?? conversation.assistantSnapshot?.name
      ?? null,
  }
}

function estimateMockContext(conversation: Conversation): ConversationContextState {
  const conversationTokens = conversation.messages.reduce(
    (sum, message) => sum + estimateTokens(message.content || ''),
    0,
  )
  const attachmentTokens = conversation.messages.reduce(
    (sum, message) => sum + (message.attachments?.filter((attachment) => attachment.type === 'image').length ?? 0) * 1200,
    0,
  )
  const systemTokens = 900
  const planText = (conversation.agent_plan_state?.plan ?? conversation.agentPlanState?.plan ?? '').trim()
  const planTokens = planText ? estimateTokens(planText) + 80 : 0
  const estimatedInputTokens = systemTokens + planTokens + conversationTokens + attachmentTokens
  const contextWindowTokens = 200_000
  const usageRatio = estimatedInputTokens / contextWindowTokens
  const summary = conversation.context_state?.summary ?? conversation.contextState?.summary ?? null
  return {
    estimated_input_tokens: estimatedInputTokens,
    context_window_tokens: contextWindowTokens,
    context_window_estimated: true,
    usage_ratio: usageRatio,
    status: summary?.stale
      ? 'stale'
      : summary
        ? 'compressed'
        : usageRatio >= 0.95
          ? 'critical'
          : usageRatio >= 0.70
            ? 'warning'
            : 'normal',
    segments: [
      { id: 'system_prompt', label: 'System prompt', estimated_tokens: systemTokens, color: '#7A7A7A' },
      { id: 'agent_plan', label: 'Agent plan', estimated_tokens: planTokens, color: '#8A724C' },
      { id: 'conversation', label: 'Conversation', estimated_tokens: conversationTokens, color: '#D07652' },
      { id: 'attachments', label: 'Attachments', estimated_tokens: attachmentTokens, color: '#6A8FBD' },
    ].filter((segment) => segment.estimated_tokens > 0),
    last_measured_at: nowSeconds(),
    last_compressed_at: summary?.created_at ?? summary?.createdAt ?? null,
    compressed_message_count: summary?.source_message_ids?.length ?? summary?.sourceMessageIds?.length ?? 0,
    summary,
  }
}

function withMockContext(conversation: Conversation): Conversation {
  const contextState = estimateMockContext(conversation)
  return {
    ...conversation,
    context_state: contextState,
    contextState,
    agent_plan_state: conversation.agent_plan_state ?? conversation.agentPlanState ?? { mode: 'act', status: 'empty', plan: null, updated_at: 0 },
    agentPlanState: conversation.agentPlanState ?? conversation.agent_plan_state ?? { mode: 'act', status: 'empty', plan: null, updated_at: 0 },
  }
}

const mockChatApi = {
  async getConversations(
    offset = 0,
    limit = 50,
    folder?: string,
    projectId?: string | null,
  ): Promise<ConversationListItem[]> {
    const conversations = loadMockConversations()
      .filter((conversation) => {
        if (projectId) {
          const conversationProjectId = conversation.project_id ?? conversation.projectId ?? null
          return conversationProjectId === projectId || (!conversationProjectId && conversation.folder === folder)
        }
        return !folder || conversation.folder === folder
      })
      .sort((a, b) => b.updated_at - a.updated_at)
    return conversations.slice(offset, offset + limit).map(toListItem)
  },

  async getConversation(conversationId: string): Promise<Conversation> {
    const conversation = loadMockConversations().find((item) => item.id === conversationId)
    if (!conversation) throw new Error('Conversation not found')
    return withMockContext(conversation)
  },

  async createConversation(
    providerId?: string,
    model?: string,
    folder?: string,
    projectId?: string | null,
    assistantId?: string | null,
  ): Promise<Conversation> {
    const now = nowSeconds()
    const assistant = assistantId
      ? loadMockAssistants().find((item) => item.id === assistantId && !item.archived && item.enabled !== false)
      : undefined
    const snapshot = assistant ? assistantSnapshot(assistant) : null
    const conversation: Conversation = {
      id: `conv_dev_${crypto.randomUUID()}`,
      title: '新对话',
      provider_id: providerId?.trim() || snapshot?.provider_id || snapshot?.providerId || 'dev-provider',
      model: model?.trim() || snapshot?.model || 'dev-model',
      messages: [],
      active_skill_id: null,
      activeSkillId: null,
      assistant_id: snapshot?.id ?? null,
      assistantId: snapshot?.id ?? null,
      assistant_snapshot: snapshot,
      assistantSnapshot: snapshot,
      created_at: now,
      updated_at: now,
      pinned: false,
      folder,
      project_id: projectId ?? null,
      projectId: projectId ?? null,
      agent_todo_state: { items: [], updated_at: 0 },
      agentTodoState: { items: [], updated_at: 0 },
      agent_plan_state: { mode: 'act', status: 'empty', plan: null, updated_at: 0 },
      agentPlanState: { mode: 'act', status: 'empty', plan: null, updated_at: 0 },
    }
    const withContext = withMockContext(conversation)
    saveMockConversations([withContext, ...loadMockConversations()])
    return withContext
  },

  async getProjects(): Promise<ChatProject[]> {
    return loadMockProjectsWithLegacyFolders()
  },

  async createProject(
    name: string,
    description?: string | null,
    color?: string | null,
    rootPath?: string | null,
  ): Promise<ChatProject> {
    const normalized = normalizeProjectName(name)
    const projects = loadMockProjectsWithLegacyFolders()
    if (projects.some((project) => project.name === normalized)) {
      throw new Error('项目名称已存在')
    }
    const now = nowSeconds()
    const project: ChatProject = {
      id: `proj_dev_${crypto.randomUUID()}`,
      name: normalized,
      description: description ?? null,
      color: color ?? null,
      root_path: rootPath ?? null,
      rootPath: rootPath ?? null,
      created_at: now,
      updated_at: now,
    }
    saveMockProjects([project, ...projects])
    return project
  },

  async updateProject(
    projectId: string,
    updates: { name?: string; description?: string | null; color?: string | null; rootPath?: string | null },
  ): Promise<ChatProject> {
    const projects = loadMockProjectsWithLegacyFolders()
    const index = projects.findIndex((project) => project.id === projectId)
    if (index < 0) throw new Error('项目不存在')
    const oldName = projects[index].name
    const nextName = updates.name === undefined ? oldName : normalizeProjectName(updates.name)
    if (nextName !== oldName && projects.some((project) => project.name === nextName)) {
      throw new Error('项目名称已存在')
    }
    const nextProject: ChatProject = {
      ...projects[index],
      name: nextName,
      description: updates.description !== undefined ? updates.description : projects[index].description,
      color: updates.color !== undefined ? updates.color : projects[index].color,
      root_path: updates.rootPath !== undefined ? updates.rootPath : (projects[index].root_path ?? projects[index].rootPath ?? null),
      rootPath: updates.rootPath !== undefined ? updates.rootPath : (projects[index].rootPath ?? projects[index].root_path ?? null),
      updated_at: nowSeconds(),
    }
    projects[index] = nextProject
    saveMockProjects(projects)

    if (nextName !== oldName) {
      const conversations = loadMockConversations().map((conversation) =>
        conversation.folder === oldName
          ? { ...conversation, folder: nextName, updated_at: nowSeconds() }
          : conversation,
      )
      saveMockConversations(conversations)
    }
    return nextProject
  },

  async openProjectFolder(projectId: string): Promise<void> {
    const project = loadMockProjectsWithLegacyFolders().find((item) => item.id === projectId)
    if (!project) throw new Error('项目不存在')
    const rootPath = (project.root_path ?? project.rootPath ?? '').trim()
    if (!rootPath) throw new Error('该项目尚未配置文件夹')
    console.info('[mock] open project folder:', rootPath)
  },

  async deleteProject(projectId: string): Promise<void> {
    const projects = loadMockProjectsWithLegacyFolders()
    const project = projects.find((item) => item.id === projectId)
    if (!project) throw new Error('项目不存在')
    saveMockProjects(projects.filter((item) => item.id !== projectId))
    saveMockConversations(
      loadMockConversations().map((conversation) =>
        (conversation.project_id ?? conversation.projectId) === project.id || conversation.folder === project.name
          ? { ...conversation, folder: undefined, project_id: null, projectId: null, updated_at: nowSeconds() }
          : conversation,
      ),
    )
  },

  async getAssistants(): Promise<ChatAssistant[]> {
    return loadMockAssistants().filter((assistant) => !assistant.archived)
  },

  async createAssistant(assistant: ChatAssistant): Promise<ChatAssistant> {
    const next = normalizeAssistant({
      ...assistant,
      id: assistant.id || `asst_dev_${crypto.randomUUID()}`,
      built_in: false,
      created_at: assistant.created_at ?? nowSeconds(),
    })
    if (!next.name) throw new Error('助手名称不能为空')
    const assistants = loadMockAssistants()
    if (assistants.some((item) => !item.archived && item.name === next.name)) {
      throw new Error('助手名称已存在')
    }
    saveMockAssistants([next, ...assistants])
    return next
  },

  async updateAssistant(assistant: ChatAssistant): Promise<ChatAssistant> {
    const assistants = loadMockAssistants()
    const index = assistants.findIndex((item) => item.id === assistant.id)
    if (index < 0) throw new Error('助手不存在')
    const next = normalizeAssistant({
      ...assistant,
      built_in: assistants[index].built_in,
      created_at: assistants[index].created_at,
    })
    if (!next.name) throw new Error('助手名称不能为空')
    if (assistants.some((item) => item.id !== next.id && !item.archived && item.name === next.name)) {
      throw new Error('助手名称已存在')
    }
    assistants[index] = next
    saveMockAssistants(assistants)
    return next
  },

  async duplicateAssistant(assistantId: string): Promise<ChatAssistant> {
    const assistants = loadMockAssistants()
    const source = assistants.find((assistant) => assistant.id === assistantId)
    if (!source) throw new Error('助手不存在')
    const baseName = `${source.name} 副本`
    let name = baseName
    let suffix = 2
    while (assistants.some((assistant) => !assistant.archived && assistant.name === name)) {
      name = `${baseName} ${suffix}`
      suffix += 1
    }
    const copy = normalizeAssistant({
      ...source,
      id: `asst_dev_${crypto.randomUUID()}`,
      name,
      built_in: false,
      created_at: nowSeconds(),
    })
    saveMockAssistants([copy, ...assistants])
    return copy
  },

  async deleteAssistant(assistantId: string): Promise<void> {
    const assistants = loadMockAssistants()
    const index = assistants.findIndex((assistant) => assistant.id === assistantId)
    if (index < 0) throw new Error('助手不存在')
    assistants[index] = {
      ...assistants[index],
      archived: true,
      updated_at: nowSeconds(),
    }
    saveMockAssistants(assistants)
  },

  async sendMessage(
    conversationId: string,
    content: string,
    attachments: PendingAttachment[] = [],
    activeSkillId?: string | null,
  ): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const now = nowSeconds()
    const conversation = { ...conversations[index] }
    conversation.active_skill_id = activeSkillId ?? conversation.active_skill_id ?? conversation.activeSkillId ?? null
    conversation.activeSkillId = conversation.active_skill_id
    conversation.messages = [
      ...conversation.messages,
      {
        id: `msg_dev_${crypto.randomUUID()}`,
        role: 'user',
        content,
        attachments: attachments.map((attachment) => ({
          id: attachment.id,
          type: attachment.type,
          name: attachment.name,
          path: attachment.path,
        })),
        timestamp: now,
      },
      {
        id: `msg_dev_${crypto.randomUUID()}`,
        role: 'assistant',
        content: '这是浏览器预览模式的本地回复。启动 Tauri 桌面应用后会调用真实模型接口。',
        active_skill_id: conversation.active_skill_id,
        timestamp: now,
      },
    ]
    const currentPlanMode = conversation.agent_plan_state?.mode ?? conversation.agentPlanState?.mode ?? 'act'
    if (currentPlanMode === 'plan') {
      const reply = conversation.messages[conversation.messages.length - 1]?.content ?? ''
      conversation.agent_plan_state = {
        mode: 'plan',
        status: 'draft',
        plan: reply,
        updated_at: now,
      }
      conversation.agentPlanState = conversation.agent_plan_state
    }
    if (conversation.title === '新对话') {
      conversation.title = content.length > 30 ? `${content.slice(0, 30)}...` : content
    }
    conversation.updated_at = now
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async setAgentPlanMode(conversationId: string, mode: AgentPlanMode): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const now = nowSeconds()
    const current = conversations[index].agent_plan_state ?? conversations[index].agentPlanState ?? {
      mode: 'act',
      status: 'empty',
      plan: null,
      updated_at: 0,
    }
    const conversation = {
      ...conversations[index],
      agent_plan_state: { ...current, mode, updated_at: now },
      updated_at: now,
    }
    conversation.agentPlanState = conversation.agent_plan_state
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async executeAgentPlan(conversationId: string): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const now = nowSeconds()
    const current = conversations[index].agent_plan_state ?? conversations[index].agentPlanState ?? {
      mode: 'act',
      status: 'empty',
      plan: null,
      updated_at: 0,
    }
    const hasPlan = Boolean(current.plan?.trim())
    const conversation = {
      ...conversations[index],
      agent_plan_state: {
        ...current,
        mode: 'act' as AgentPlanMode,
        status: hasPlan ? 'approved' as const : 'empty' as const,
        updated_at: now,
      },
      updated_at: now,
    }
    conversation.agentPlanState = conversation.agent_plan_state
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async deleteConversation(conversationId: string): Promise<void> {
    saveMockConversations(loadMockConversations().filter((item) => item.id !== conversationId))
  },

  async updateConversation(
    conversationId: string,
    updates: {
      title?: string
      pinned?: boolean
      folder?: string
      projectId?: string | null
      providerId?: string
      model?: string
      activeSkillId?: string | null
      assistantId?: string | null
    }
  ): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const hasFolderUpdate = Object.prototype.hasOwnProperty.call(updates, 'folder')
    const hasProjectUpdate = Object.prototype.hasOwnProperty.call(updates, 'projectId')
    const project = hasProjectUpdate && updates.projectId
      ? loadMockProjectsWithLegacyFolders().find((item) => item.id === updates.projectId)
      : undefined
    const conversation = {
      ...conversations[index],
      title: updates.title ?? conversations[index].title,
      pinned: updates.pinned ?? conversations[index].pinned,
      folder: hasProjectUpdate
        ? project?.name
        : hasFolderUpdate
          ? updates.folder || undefined
          : conversations[index].folder,
      project_id: hasProjectUpdate
        ? updates.projectId || null
        : conversations[index].project_id ?? conversations[index].projectId ?? null,
      projectId: hasProjectUpdate
        ? updates.projectId || null
        : conversations[index].projectId ?? conversations[index].project_id ?? null,
      provider_id: updates.providerId ?? conversations[index].provider_id,
      model: updates.model ?? conversations[index].model,
      active_skill_id:
        updates.activeSkillId !== undefined
          ? updates.activeSkillId || null
          : conversations[index].active_skill_id ?? conversations[index].activeSkillId ?? null,
      updated_at: nowSeconds(),
    }
    if (Object.prototype.hasOwnProperty.call(updates, 'assistantId')) {
      const assistantId = updates.assistantId?.trim() ?? ''
      if (!assistantId) {
        conversation.assistant_id = null
        conversation.assistantId = null
        conversation.assistant_snapshot = null
        conversation.assistantSnapshot = null
      } else {
        const assistant = loadMockAssistants().find((item) =>
          item.id === assistantId && !item.archived && item.enabled !== false
        )
        if (!assistant) throw new Error('助手不存在或不可用')
        const snapshot = assistantSnapshot(assistant)
        conversation.assistant_id = snapshot.id
        conversation.assistantId = snapshot.id
        conversation.assistant_snapshot = snapshot
        conversation.assistantSnapshot = snapshot
        conversation.active_skill_id = null
      }
    }
    conversation.activeSkillId = conversation.active_skill_id
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async updateMessage(
    conversationId: string,
    messageId: string,
    content: string,
  ): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const trimmed = content.trim()
    if (!trimmed) throw new Error('消息内容不能为空')
    const conversation = { ...conversations[index] }
    const messageIndex = conversation.messages.findIndex((message) => message.id === messageId)
    if (messageIndex < 0) throw new Error('Message not found')
    if (conversation.messages[messageIndex].role !== 'assistant') {
      throw new Error('仅支持编辑助手回复')
    }
    conversation.messages = conversation.messages.map((message, i) =>
      i === messageIndex
        ? { ...message, content: trimmed, timestamp: nowSeconds() }
        : message,
    )
    conversation.updated_at = nowSeconds()
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async deleteMessage(conversationId: string, messageId: string): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const conversation = { ...conversations[index] }
    const target = conversation.messages.find((message) => message.id === messageId)
    if (!target) throw new Error('Message not found')
    if (target.role !== 'assistant') throw new Error('仅支持删除助手回复')
    conversation.messages = conversation.messages.filter((message) => message.id !== messageId)
    conversation.updated_at = nowSeconds()
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async regenerateMessage(conversationId: string, messageId: string): Promise<Conversation> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const conversation = { ...conversations[index] }
    const messageIndex = conversation.messages.findIndex((message) => message.id === messageId)
    if (messageIndex < 0) throw new Error('Message not found')
    if (conversation.messages[messageIndex].role !== 'assistant') {
      throw new Error('仅支持重新生成助手回复')
    }
    const kept = conversation.messages.slice(0, messageIndex)
    const lastUser = kept[kept.length - 1]
    if (!lastUser || lastUser.role !== 'user') {
      throw new Error('缺少对应的用户消息，无法重新生成')
    }
    conversation.messages = [
      ...kept,
      {
        id: `msg_dev_${crypto.randomUUID()}`,
        role: 'assistant',
        content: `（重新生成预览）${lastUser.content.slice(0, 80)}`,
        timestamp: nowSeconds(),
      },
    ]
    conversation.updated_at = nowSeconds()
    const contextState = estimateMockContext(conversation)
    conversation.context_state = contextState
    conversation.contextState = contextState
    conversations[index] = conversation
    saveMockConversations(conversations)
    return conversation
  },

  async getContextStats(conversationId: string): Promise<{ contextState: ConversationContextState; conversation: Conversation }> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const conversation = withMockContext(conversations[index])
    conversations[index] = conversation
    saveMockConversations(conversations)
    return { contextState: conversation.context_state ?? {}, conversation }
  },

  async compressContext(conversationId: string): Promise<{ contextState: ConversationContextState; conversation: Conversation }> {
    const conversations = loadMockConversations()
    const index = conversations.findIndex((item) => item.id === conversationId)
    if (index < 0) throw new Error('Conversation not found')
    const conversation = { ...conversations[index] }
    const cutoff = Math.max(0, conversation.messages.length - 8)
    const source = conversation.messages.slice(0, cutoff)
    if (source.length < 2) {
      throw new Error('没有足够的旧消息可以压缩')
    }
    const summary = {
      id: `ctxsum_dev_${crypto.randomUUID()}`,
      content: `Browser preview summary for ${source.length} older messages.`,
      source_message_ids: source.map((message) => message.id),
      source_until_message_id: source[source.length - 1]?.id ?? '',
      token_estimate_before: source.reduce((sum, message) => sum + estimateTokens(message.content || ''), 0),
      token_estimate_after: 20,
      created_at: nowSeconds(),
      provider_id: conversation.provider_id,
      model: conversation.model,
      stale: false,
    }
    const baseState = estimateMockContext(conversation)
    conversation.context_state = {
      ...baseState,
      status: 'compressed',
      summary,
      last_compressed_at: summary.created_at,
      compressed_message_count: source.length,
      segments: [
        ...(baseState.segments ?? []).filter((segment) => segment.id !== 'summarized_conversation'),
        { id: 'summarized_conversation', label: 'Summarized conversation', estimated_tokens: 20, color: '#BF3F66' },
      ],
    }
    conversation.contextState = conversation.context_state
    conversations[index] = conversation
    saveMockConversations(conversations)
    return { contextState: conversation.context_state, conversation }
  },
}

export const chatApi = {
  // 获取对话列表
  async getConversations(
    offset = 0,
    limit = 50,
    folder?: string,
    projectId?: string | null,
  ): Promise<ConversationListItem[]> {
    if (!isTauriRuntime()) return mockChatApi.getConversations(offset, limit, folder, projectId)
    const result = await invoke<{ success: boolean; conversations: ConversationListItem[] }>(
      'chat_get_conversations',
      { offset, limit, folder, projectId }
    )
    if (!result.success) {
      throw new Error('Failed to get conversations')
    }
    return result.conversations
  },

  // 获取对话详情
  async getConversation(conversationId: string): Promise<Conversation> {
    if (!isTauriRuntime()) return mockChatApi.getConversation(conversationId)
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_get_conversation',
      { conversationId }
    )
    if (!result.success) {
      throw new Error('Failed to get conversation')
    }
    return result.conversation
  },

  // 创建新对话
  async createConversation(
    providerId?: string,
    model?: string,
    folder?: string,
    projectId?: string | null,
    assistantId?: string | null,
  ): Promise<Conversation> {
    if (!isTauriRuntime()) return mockChatApi.createConversation(providerId, model, folder, projectId, assistantId)
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_create_conversation',
      { providerId, model, folder, projectId, assistantId }
    )
    if (!result.success) {
      throw new Error('Failed to create conversation')
    }
    return result.conversation
  },

  // 创建「对话搭建专家」会话(瞬态搭建助手,只暴露 save_assistant 工具)
  async createBuilderConversation(
    providerId?: string,
    model?: string,
    projectId?: string | null,
  ): Promise<Conversation> {
    if (!isTauriRuntime()) {
      // dev 浏览器无后端 LLM,搭建流程只在 Tauri 内真正可用;这里仅返回一个占位会话。
      return mockChatApi.createConversation(providerId, model, undefined, projectId, null)
    }
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_create_builder_conversation',
      { providerId, model, projectId }
    )
    if (!result.success) {
      throw new Error('Failed to create builder conversation')
    }
    return result.conversation
  },

  async getProjects(): Promise<ChatProject[]> {
    if (!isTauriRuntime()) return mockChatApi.getProjects()
    const result = await invoke<{ success: boolean; projects: ChatProject[] }>(
      'chat_get_projects',
    )
    if (!result.success) {
      throw new Error('Failed to get projects')
    }
    return result.projects
  },

  async createProject(
    name: string,
    description?: string | null,
    color?: string | null,
    rootPath?: string | null,
  ): Promise<ChatProject> {
    if (!isTauriRuntime()) return mockChatApi.createProject(name, description, color, rootPath)
    const result = await invoke<{ success: boolean; project: ChatProject }>(
      'chat_create_project',
      { name, description, color, rootPath },
    )
    if (!result.success) {
      throw new Error('Failed to create project')
    }
    return result.project
  },

  async updateProject(
    projectId: string,
    updates: { name?: string; description?: string | null; color?: string | null; rootPath?: string | null },
  ): Promise<ChatProject> {
    if (!isTauriRuntime()) return mockChatApi.updateProject(projectId, updates)
    const hasDescriptionUpdate = Object.prototype.hasOwnProperty.call(updates, 'description')
    const hasColorUpdate = Object.prototype.hasOwnProperty.call(updates, 'color')
    const hasRootPathUpdate = Object.prototype.hasOwnProperty.call(updates, 'rootPath')
    const result = await invoke<{ success: boolean; project: ChatProject }>(
      'chat_update_project',
      {
        projectId,
        name: updates.name,
        description: hasDescriptionUpdate ? updates.description : undefined,
        descriptionSet: hasDescriptionUpdate,
        color: hasColorUpdate ? updates.color : undefined,
        colorSet: hasColorUpdate,
        rootPath: hasRootPathUpdate ? updates.rootPath : undefined,
        rootPathSet: hasRootPathUpdate,
      },
    )
    if (!result.success) {
      throw new Error('Failed to update project')
    }
    return result.project
  },

  async deleteProject(projectId: string): Promise<void> {
    if (!isTauriRuntime()) return mockChatApi.deleteProject(projectId)
    const result = await invoke<{ success: boolean }>('chat_delete_project', { projectId })
    if (!result.success) {
      throw new Error('Failed to delete project')
    }
  },

  async openProjectFolder(projectId: string): Promise<void> {
    if (!isTauriRuntime()) return mockChatApi.openProjectFolder(projectId)
    const result = await invoke<{ success: boolean; error?: string }>(
      'chat_project_open_folder',
      { projectId },
    )
    if (!result.success) {
      throw new Error('打开项目文件夹失败')
    }
  },

  async getAssistants(): Promise<ChatAssistant[]> {
    if (!isTauriRuntime()) return mockChatApi.getAssistants()
    const result = await invoke<{ success: boolean; assistants: ChatAssistant[] }>(
      'chat_get_assistants',
    )
    if (!result.success) {
      throw new Error('Failed to get assistants')
    }
    return result.assistants
  },

  async createAssistant(assistant: ChatAssistant): Promise<ChatAssistant> {
    if (!isTauriRuntime()) return mockChatApi.createAssistant(assistant)
    const result = await invoke<{ success: boolean; assistant: ChatAssistant }>(
      'chat_create_assistant',
      { assistant },
    )
    if (!result.success) {
      throw new Error('Failed to create assistant')
    }
    return result.assistant
  },

  async updateAssistant(assistant: ChatAssistant): Promise<ChatAssistant> {
    if (!isTauriRuntime()) return mockChatApi.updateAssistant(assistant)
    const result = await invoke<{ success: boolean; assistant: ChatAssistant }>(
      'chat_update_assistant',
      { assistant },
    )
    if (!result.success) {
      throw new Error('Failed to update assistant')
    }
    return result.assistant
  },

  async duplicateAssistant(assistantId: string): Promise<ChatAssistant> {
    if (!isTauriRuntime()) return mockChatApi.duplicateAssistant(assistantId)
    const result = await invoke<{ success: boolean; assistant: ChatAssistant }>(
      'chat_duplicate_assistant',
      { assistantId },
    )
    if (!result.success) {
      throw new Error('Failed to duplicate assistant')
    }
    return result.assistant
  },

  async deleteAssistant(assistantId: string): Promise<void> {
    if (!isTauriRuntime()) return mockChatApi.deleteAssistant(assistantId)
    const result = await invoke<{ success: boolean }>('chat_delete_assistant', { assistantId })
    if (!result.success) {
      throw new Error('Failed to delete assistant')
    }
  },

  // 发送消息
  async sendMessage(
    conversationId: string,
    content: string,
    attachments: PendingAttachment[] = [],
    activeSkillId?: string | null,
  ): Promise<Conversation> {
    if (!isTauriRuntime()) {
      return mockChatApi.sendMessage(conversationId, content, attachments, activeSkillId)
    }
    const result = await invoke<{ success: boolean; conversation?: Conversation; error?: string }>(
      'chat_send_message',
      {
        conversationId,
        content,
        attachments: attachments.map((attachment) => attachment.path),
        activeSkillId,
      }
    )
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to send message')
    }
    return result.conversation
  },

  // 删除对话
  async deleteConversation(conversationId: string): Promise<void> {
    if (!isTauriRuntime()) return mockChatApi.deleteConversation(conversationId)
    const result = await invoke<{ success: boolean }>('chat_delete_conversation', {
      conversationId,
    })
    if (!result.success) {
      throw new Error('Failed to delete conversation')
    }
  },

  // 更新对话
  async updateConversation(
    conversationId: string,
    updates: {
      title?: string
      pinned?: boolean
      folder?: string
      projectId?: string | null
      providerId?: string
      model?: string
      activeSkillId?: string | null
      assistantId?: string | null
    }
  ): Promise<Conversation> {
    if (!isTauriRuntime()) return mockChatApi.updateConversation(conversationId, updates)
    const hasFolderUpdate = Object.prototype.hasOwnProperty.call(updates, 'folder')
    const hasProjectUpdate = Object.prototype.hasOwnProperty.call(updates, 'projectId')
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_update_conversation',
      {
        conversationId,
        title: updates.title,
        pinned: updates.pinned,
        folder: hasFolderUpdate ? updates.folder ?? '' : undefined,
        projectId: hasProjectUpdate ? updates.projectId ?? '' : undefined,
        providerId: updates.providerId,
        model: updates.model,
        activeSkillId: updates.activeSkillId,
        assistantId: updates.assistantId,
      }
    )
    if (!result.success) {
      throw new Error('Failed to update conversation')
    }
    return result.conversation
  },

  async updateMessage(
    conversationId: string,
    messageId: string,
    content: string,
  ): Promise<Conversation> {
    if (!isTauriRuntime()) {
      return mockChatApi.updateMessage(conversationId, messageId, content)
    }
    const result = await invoke<{
      success: boolean
      conversation?: Conversation
      error?: string
    }>('chat_update_message', { conversationId, messageId, content })
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to update message')
    }
    return result.conversation
  },

  async deleteMessage(conversationId: string, messageId: string): Promise<Conversation> {
    if (!isTauriRuntime()) {
      return mockChatApi.deleteMessage(conversationId, messageId)
    }
    const result = await invoke<{
      success: boolean
      conversation?: Conversation
      error?: string
    }>('chat_delete_message', { conversationId, messageId })
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to delete message')
    }
    return result.conversation
  },

  async regenerateMessage(conversationId: string, messageId: string): Promise<Conversation> {
    if (!isTauriRuntime()) {
      return mockChatApi.regenerateMessage(conversationId, messageId)
    }
    const result = await invoke<{
      success: boolean
      conversation?: Conversation
      error?: string
    }>('chat_regenerate_message', { conversationId, messageId })
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to regenerate message')
    }
    return result.conversation
  },

  async getContextStats(conversationId: string): Promise<{ contextState: ConversationContextState; conversation: Conversation }> {
    if (!isTauriRuntime()) return mockChatApi.getContextStats(conversationId)
    const result = await invoke<{
      success: boolean
      contextState?: ConversationContextState
      conversation?: Conversation
      error?: string
    }>('chat_get_context_stats', { conversationId })
    if (!result.success || !result.contextState || !result.conversation) {
      throw new Error(result.error || 'Failed to get context stats')
    }
    return { contextState: result.contextState, conversation: result.conversation }
  },

  async compressContext(conversationId: string): Promise<{ contextState: ConversationContextState; conversation: Conversation }> {
    if (!isTauriRuntime()) return mockChatApi.compressContext(conversationId)
    const result = await invoke<{
      success: boolean
      contextState?: ConversationContextState
      conversation?: Conversation
      error?: string
    }>('chat_compress_context', { conversationId })
    if (!result.success || !result.contextState || !result.conversation) {
      throw new Error(result.error || 'Failed to compress context')
    }
    return { contextState: result.contextState, conversation: result.conversation }
  },

  async setAgentPlanMode(conversationId: string, mode: AgentPlanMode): Promise<Conversation> {
    if (!isTauriRuntime()) return mockChatApi.setAgentPlanMode(conversationId, mode)
    const result = await invoke<{ success: boolean; conversation?: Conversation; error?: string }>(
      'chat_set_agent_plan_mode',
      { conversationId, mode },
    )
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to set plan mode')
    }
    return result.conversation
  },

  async executeAgentPlan(conversationId: string): Promise<Conversation> {
    if (!isTauriRuntime()) return mockChatApi.executeAgentPlan(conversationId)
    const result = await invoke<{ success: boolean; conversation?: Conversation; error?: string }>(
      'chat_execute_agent_plan',
      { conversationId },
    )
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to execute plan')
    }
    return result.conversation
  },

  async cancelStream(conversationId: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke<void>('chat_cancel_stream', { conversationId })
  },
}
