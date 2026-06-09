import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  ArrowUp,
  Archive,
  Check,
  ChevronDown,
  ChevronRight,
  CircleHelp,
  Eraser,
  Folder,
  FolderPlus,
  ListChecks,
  MessageSquarePlus,
  Paperclip,
  Play,
  Plus,
  Search,
  Settings,
  ShieldAlert,
  SlidersHorizontal,
  Square,
  Wrench,
} from 'lucide-react'
import { ChatAttachments } from './ChatAttachments'
import { api, type ChatToolDefinition } from '../api/tauri'
import { chatApi } from './api'
import type { AgentPlanMode, AgentPlanState, ChatProject, PendingAttachment } from './types'

const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'tiff', 'tif', 'heic', 'heif']
const isTauriRuntime = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

function isAttachableClipboardFile(file: File): boolean {
  return Boolean(file.name?.trim()) || file.size > 0
}

function undoAccidentalFilenamePaste(
  textarea: HTMLTextAreaElement,
  valueBeforePaste: string,
  clipText: string,
  selectionStart: number,
  selectionEnd: number,
  setValue: (value: string) => void,
) {
  if (!clipText.trim()) return

  const currentValue = textarea.value
  const expectedAfterPaste = `${valueBeforePaste.slice(0, selectionStart)}${clipText}${valueBeforePaste.slice(selectionEnd)}`
  if (currentValue !== expectedAfterPaste) return

  const cleaned = `${valueBeforePaste.slice(0, selectionStart)}${valueBeforePaste.slice(selectionEnd)}`
  setValue(cleaned)
  requestAnimationFrame(() => {
    textarea.value = cleaned
    textarea.selectionStart = selectionStart
    textarea.selectionEnd = selectionStart
    textarea.style.height = 'auto'
    textarea.style.height = `${Math.min(textarea.scrollHeight, 160)}px`
  })
}

function shouldComposerAutoFocus(activeElement: Element | null): boolean {
  if (!activeElement || activeElement === document.body || activeElement === document.documentElement) {
    return true
  }
  if (activeElement instanceof HTMLTextAreaElement || activeElement instanceof HTMLInputElement) {
    return false
  }
  return activeElement.closest('[data-chat-composer="true"]') !== null
}

function isExternalMcpTool(tool: ChatToolDefinition): boolean {
  return tool.source !== 'skill' && tool.source !== 'native'
}

function projectPathLabel(project: ChatProject): string {
  const rootPath = project.root_path ?? project.rootPath ?? ''
  if (!rootPath) return ''
  const normalized = rootPath.replace(/\\/g, '/')
  return normalized.split('/').filter(Boolean).pop() ?? rootPath
}

function pathTail(path: string): string {
  const normalized = path.replace(/\\/g, '/')
  return normalized.split('/').filter(Boolean).pop() ?? path
}

function projectUpdatedAt(project: ChatProject): number {
  return project.updated_at ?? project.updatedAt ?? project.created_at ?? project.createdAt ?? 0
}

function nextBlankProjectName(projects: ChatProject[]): string {
  const names = new Set(projects.map((project) => project.name))
  if (!names.has('新项目')) return '新项目'
  for (let index = 2; index < 100; index += 1) {
    const name = `新项目 ${index}`
    if (!names.has(name)) return name
  }
  return `新项目 ${Date.now()}`
}

const APPROVAL_POLICY_OPTIONS = [
  {
    value: 'always_confirm',
    label: '每次确认',
    title: '请求批准',
    description: '所有工具调用都先问你',
  },
  {
    value: 'readonly_auto_sensitive_confirm',
    label: '敏感确认',
    title: '替我审批',
    description: '只对写文件、终端等风险操作确认',
  },
  {
    value: 'auto',
    label: '完全访问',
    title: '完全访问权限',
    description: '工具调用自动放行',
  },
]

type SlashCommandId =
  | 'help'
  | 'plan'
  | 'new'
  | 'compact'
  | 'clear'
  | 'settings'
  | 'tools'
  | 'attach'

interface SlashCommandDefinition {
  id: SlashCommandId
  slash: `/${string}`
  title: string
  description: string
  category: string
  keywords: string[]
}

interface ActiveSlashToken {
  start: number
  end: number
  query: string
}

const LOCAL_SLASH_COMMANDS: SlashCommandDefinition[] = [
  {
    id: 'help',
    slash: '/help',
    title: '/help',
    description: 'Show commands',
    category: 'Local',
    keywords: ['help', 'commands', '帮助', '命令'],
  },
  {
    id: 'plan',
    slash: '/plan',
    title: '/plan',
    description: 'Enter plan mode',
    category: 'Local',
    keywords: ['plan', 'act', 'mode', '计划', '模式', '切换'],
  },
  {
    id: 'new',
    slash: '/new',
    title: '/new',
    description: 'Start a new chat',
    category: 'Local',
    keywords: ['new', 'chat', 'conversation', '新建', '新对话'],
  },
  {
    id: 'compact',
    slash: '/compact',
    title: '/compact',
    description: 'Compress context',
    category: 'Local',
    keywords: ['compact', 'compress', 'context', '压缩', '上下文'],
  },
  {
    id: 'clear',
    slash: '/clear',
    title: '/clear',
    description: 'Clear current chat',
    category: 'Local',
    keywords: ['clear', 'delete', 'reset', '清空', '删除', '重置'],
  },
  {
    id: 'settings',
    slash: '/settings',
    title: '/settings',
    description: 'Open chat settings',
    category: 'Local',
    keywords: ['settings', 'config', '设置', '配置'],
  },
  {
    id: 'tools',
    slash: '/tools',
    title: '/tools',
    description: 'Show tool status',
    category: 'Local',
    keywords: ['tools', 'mcp', 'skill', '工具', '技能'],
  },
  {
    id: 'attach',
    slash: '/attach',
    title: '/attach',
    description: 'Add files or images',
    category: 'Local',
    keywords: ['attach', 'file', 'image', '附件', '文件', '图片'],
  },
]

function slashCommandIcon(commandId: SlashCommandId) {
  switch (commandId) {
    case 'help':
      return CircleHelp
    case 'plan':
      return ListChecks
    case 'new':
      return MessageSquarePlus
    case 'compact':
      return Archive
    case 'clear':
      return Eraser
    case 'settings':
      return Settings
    case 'tools':
      return Wrench
    case 'attach':
      return Paperclip
  }
}

function findActiveSlashToken(value: string, cursor: number): ActiveSlashToken | null {
  if (cursor < 0 || cursor > value.length) return null

  let start = cursor
  while (start > 0 && !/\s/.test(value[start - 1])) {
    start -= 1
  }

  const token = value.slice(start, cursor)
  if (!token.startsWith('/')) return null
  if (start > 0 && !/\s/.test(value[start - 1])) return null
  if (token.slice(1).includes('/')) return null

  return {
    start,
    end: cursor,
    query: token.slice(1),
  }
}

function commandMatches(command: SlashCommandDefinition, query: string): boolean {
  const normalized = query.trim().toLowerCase()
  if (!normalized) return true

  const searchable = [
    command.slash.slice(1),
    command.title,
    command.description,
    command.category,
    ...command.keywords,
  ].map((item) => item.toLowerCase())

  return searchable.some((item) => item.includes(normalized))
}

function approvalPolicyOption(policy?: string) {
  return APPROVAL_POLICY_OPTIONS.find((option) => option.value === policy)
    ?? APPROVAL_POLICY_OPTIONS[1]
}

function imageExtensionForMime(mimeType: string): string {
  switch (mimeType.toLowerCase()) {
    case 'image/jpeg':
      return 'jpg'
    case 'image/gif':
      return 'gif'
    case 'image/webp':
      return 'webp'
    case 'image/bmp':
      return 'bmp'
    case 'image/tiff':
      return 'tiff'
    case 'image/heic':
      return 'heic'
    case 'image/heif':
      return 'heif'
    case 'image/png':
    default:
      return 'png'
  }
}

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : ''
      resolve(result.split(',')[1] ?? '')
    }
    reader.onerror = () => reject(reader.error ?? new Error('读取剪贴板图片失败'))
    reader.readAsDataURL(file)
  })
}

interface InputBarProps {
  onSend: (content: string, attachments: PendingAttachment[]) => void
  disabled?: boolean
  onCancel?: () => void
  cancelVisible?: boolean
  cancelling?: boolean
  onOpenSettings?: () => void
  onOpenTools?: () => void
  onNewChat?: () => void | Promise<void>
  onCompactContext?: () => void | Promise<void>
  onClearChat?: () => void | Promise<void>
  enabledTools?: ChatToolDefinition[]
  toolsDisabledReason?: string
  toolStatusHint?: string
  sendDisabledReason?: string
  approvalPolicy?: string
  onApprovalPolicyChange?: (approvalPolicy: string) => void | Promise<void>
  agentPlanState?: AgentPlanState | null
  onAgentPlanModeChange?: (mode: AgentPlanMode) => void | Promise<void>
  onExecuteAgentPlan?: () => void | Promise<void>
  enabledSkills?: { id: string; name: string }[]
  onOpenSkillSettings?: () => void
  selectedProject?: ChatProject | null
  onSelectProject?: (project: ChatProject | null) => void | Promise<void>
  showProjectEntry?: boolean
  autoFocus?: boolean
  /** footer：贴底（有消息时）；inline：嵌入居中区域（空对话欢迎页） */
  layout?: 'footer' | 'inline'
}

export function InputBar({
  onSend,
  disabled,
  onCancel,
  cancelVisible,
  cancelling,
  onOpenSettings,
  onOpenTools,
  onNewChat,
  onCompactContext,
  onClearChat,
  enabledTools = [],
  toolsDisabledReason,
  toolStatusHint,
  sendDisabledReason,
  approvalPolicy,
  onApprovalPolicyChange,
  agentPlanState = null,
  onAgentPlanModeChange,
  onExecuteAgentPlan,
  enabledSkills = [],
  onOpenSkillSettings,
  selectedProject = null,
  onSelectProject,
  showProjectEntry = false,
  autoFocus,
  layout = 'footer',
}: InputBarProps) {
  const [input, setInput] = useState('')
  const [attachments, setAttachments] = useState<PendingAttachment[]>([])
  const [attachmentError, setAttachmentError] = useState('')
  const [dragActive, setDragActive] = useState(false)
  const [toolPanelOpen, setToolPanelOpen] = useState(false)
  const [projectMenuOpen, setProjectMenuOpen] = useState(false)
  const [projectOptions, setProjectOptions] = useState<ChatProject[]>([])
  const [projectOptionsLoading, setProjectOptionsLoading] = useState(false)
  const [projectOptionsError, setProjectOptionsError] = useState('')
  const [projectSearchQuery, setProjectSearchQuery] = useState('')
  const [projectCreating, setProjectCreating] = useState(false)
  const [projectCreateMenuOpen, setProjectCreateMenuOpen] = useState(false)
  const [slashPanelOpen, setSlashPanelOpen] = useState(false)
  const [slashSelectedIndex, setSlashSelectedIndex] = useState(0)
  const [activeSlashToken, setActiveSlashToken] = useState<ActiveSlashToken | null>(null)
  const [slashPanelLeft, setSlashPanelLeft] = useState(0)
  const innerRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const agentPlanMode = agentPlanState?.mode ?? 'act'
  const agentPlanText = agentPlanState?.plan?.trim() ?? ''
  const agentPlanActive = agentPlanMode === 'plan'
  const projectEntryEnabled = Boolean(showProjectEntry && onSelectProject)

  const closeProjectMenu = useCallback(() => {
    setProjectMenuOpen(false)
    setProjectCreateMenuOpen(false)
  }, [])

  const attachmentsFromPaths = useCallback(
    (paths: string[]) =>
      paths.map((path) => {
        const normalized = path.replace(/\\/g, '/')
        const name = normalized.split('/').filter(Boolean).pop() || '附件'
        const ext = name.split('.').pop()?.toLowerCase() ?? ''
        const type: PendingAttachment['type'] = IMAGE_EXTENSIONS.includes(ext) ? 'image' : 'file'
        return {
          id: `pending-att-${crypto.randomUUID()}`,
          type,
          name,
          path,
        }
      }),
    [],
  )

  const loadProjectOptions = useCallback(async () => {
    if (!projectEntryEnabled) return
    setProjectOptionsLoading(true)
    setProjectOptionsError('')
    try {
      setProjectOptions(await chatApi.getProjects())
    } catch (err) {
      console.error('Failed to load chat projects:', err)
      setProjectOptionsError(typeof err === 'string' ? err : err instanceof Error ? err.message : '项目加载失败')
    } finally {
      setProjectOptionsLoading(false)
    }
  }, [projectEntryEnabled])

  const toggleProjectMenu = useCallback(() => {
    if (!projectEntryEnabled || disabled) return
    setSlashPanelOpen(false)
    setToolPanelOpen(false)
    setProjectMenuOpen((open) => {
      const nextOpen = !open
      setProjectCreateMenuOpen(false)
      if (nextOpen) {
        setProjectSearchQuery('')
        void loadProjectOptions()
      }
      return nextOpen
    })
  }, [disabled, loadProjectOptions, projectEntryEnabled])

  const selectProject = useCallback(async (project: ChatProject | null) => {
    if (!onSelectProject) return
    closeProjectMenu()
    await onSelectProject(project)
    requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }))
  }, [closeProjectMenu, onSelectProject])

  const createBlankProject = useCallback(async () => {
    if (!onSelectProject || disabled || projectCreating) return
    setProjectOptionsError('')
    setProjectCreating(true)
    try {
      const project = await chatApi.createProject(nextBlankProjectName(projectOptions), null, null, null)
      setProjectOptions((prev) => [
        project,
        ...prev.filter((item) => item.id !== project.id),
      ])
      closeProjectMenu()
      await onSelectProject(project)
    } catch (err) {
      console.error('Failed to create blank chat project from input bar:', err)
      setProjectOptionsError(typeof err === 'string' ? err : err instanceof Error ? err.message : '项目创建失败')
    } finally {
      setProjectCreating(false)
      requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }))
    }
  }, [closeProjectMenu, disabled, onSelectProject, projectCreating, projectOptions])

  const createProjectFromFolder = useCallback(async () => {
    if (!onSelectProject || disabled || projectCreating) return
    setProjectOptionsError('')
    setProjectCreating(true)
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: '选择项目文件夹',
      })
      const rootPath = Array.isArray(picked) ? picked[0] : picked
      if (!rootPath) return
      const project = await chatApi.createProject(pathTail(rootPath), null, null, rootPath)
      setProjectOptions((prev) => [
        project,
        ...prev.filter((item) => item.id !== project.id),
      ])
      closeProjectMenu()
      await onSelectProject(project)
    } catch (err) {
      console.error('Failed to create chat project from input bar:', err)
      setProjectOptionsError(typeof err === 'string' ? err : err instanceof Error ? err.message : '项目创建失败')
    } finally {
      setProjectCreating(false)
      requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }))
    }
  }, [closeProjectMenu, disabled, onSelectProject, projectCreating])

  const updateTextareaHeight = useCallback(() => {
    const textarea = textareaRef.current
    if (!textarea) return
    textarea.style.height = 'auto'
    textarea.style.height = `${Math.min(textarea.scrollHeight, 160)}px`
  }, [])

  const syncSlashToken = useCallback((value: string, cursor: number) => {
    const token = findActiveSlashToken(value, cursor)
    setActiveSlashToken(token)
    if (token) {
      setSlashPanelOpen(true)
      setToolPanelOpen(false)
      closeProjectMenu()
    } else {
      setSlashPanelOpen(false)
    }
  }, [closeProjectMenu])

  const filteredSlashCommands = useMemo(
    () => LOCAL_SLASH_COMMANDS.filter((command) => (
      commandMatches(command, activeSlashToken?.query ?? '')
    )),
    [activeSlashToken?.query],
  )
  const visibleProjectOptions = useMemo(() => {
    const query = projectSearchQuery.trim().toLowerCase()
    return [...projectOptions]
      .sort((a, b) => projectUpdatedAt(b) - projectUpdatedAt(a))
      .filter((project) => {
        if (!query) return true
        const rootPath = project.root_path ?? project.rootPath ?? ''
        return project.name.toLowerCase().includes(query) || rootPath.toLowerCase().includes(query)
      })
      .slice(0, 8)
  }, [projectOptions, projectSearchQuery])

  const removeActiveSlashToken = useCallback(() => {
    const token = activeSlashToken
    if (!token) {
      setInput('')
      requestAnimationFrame(updateTextareaHeight)
      return
    }

    setInput((prev) => {
      const next = `${prev.slice(0, token.start)}${prev.slice(token.end)}`.replace(/^\s+/, '')
      requestAnimationFrame(() => {
        const textarea = textareaRef.current
        if (!textarea) return
        textarea.selectionStart = Math.min(token.start, next.length)
        textarea.selectionEnd = Math.min(token.start, next.length)
        updateTextareaHeight()
      })
      return next
    })
  }, [activeSlashToken, updateTextareaHeight])

  const completeActiveSlashToken = useCallback((command: SlashCommandDefinition) => {
    const token = activeSlashToken
    if (!token) return

    const cursor = token.start + command.slash.length
    setInput((prev) => {
      const next = `${prev.slice(0, token.start)}${command.slash}${prev.slice(token.end)}`
      requestAnimationFrame(() => {
        const textarea = textareaRef.current
        if (!textarea) return
        textarea.focus({ preventScroll: true })
        textarea.selectionStart = cursor
        textarea.selectionEnd = cursor
        updateTextareaHeight()
      })
      return next
    })
    setActiveSlashToken({
      start: token.start,
      end: cursor,
      query: command.slash.slice(1),
    })
    setSlashPanelOpen(true)
  }, [activeSlashToken, updateTextareaHeight])

  const selectedSlashCommand = filteredSlashCommands[slashSelectedIndex]
    ?? filteredSlashCommands[0]

  const addAttachments = useCallback(
    (next: PendingAttachment[], options?: { imagesOnly?: boolean }) => {
      const filtered = options?.imagesOnly
        ? next.filter((attachment) => attachment.type === 'image')
        : next.filter((attachment) => attachment.name.trim() !== '')
      if (filtered.length === 0) {
        setAttachmentError(options?.imagesOnly ? '请拖入图片文件' : '未识别到可添加的文件')
        return
      }

      setAttachments((prev) => {
        const existing = new Set(prev.map((attachment) => attachment.path))
        const dedupedNext = filtered.filter((attachment) => {
          if (existing.has(attachment.path)) return false
          existing.add(attachment.path)
          return true
        })
        if (dedupedNext.length === 0) {
          setAttachmentError('附件已添加')
          return prev
        }
        setAttachmentError('')
        return [...prev, ...dedupedNext]
      })
      textareaRef.current?.focus()
    },
    [],
  )

  const setAgentPlanMode = useCallback(async (mode: AgentPlanMode) => {
    if (disabled || !onAgentPlanModeChange) return
    setSlashPanelOpen(false)
    setToolPanelOpen(false)
    closeProjectMenu()
    if (agentPlanMode !== mode) {
      await onAgentPlanModeChange(mode)
    }
    requestAnimationFrame(() => {
      textareaRef.current?.focus({ preventScroll: true })
    })
  }, [agentPlanMode, closeProjectMenu, disabled, onAgentPlanModeChange])

  const toggleAgentPlanMode = useCallback(async () => {
    await setAgentPlanMode(agentPlanMode === 'plan' ? 'act' : 'plan')
  }, [agentPlanMode, setAgentPlanMode])

  const openAttachmentPicker = useCallback(async () => {
    if (disabled) return
    setToolPanelOpen(false)
    closeProjectMenu()
    setSlashPanelOpen(false)
    setAttachmentError('')
    try {
      const selected = await open({
        multiple: true,
        directory: false,
      })
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : []
      if (paths.length === 0) return

      addAttachments(attachmentsFromPaths(paths))
    } catch (err) {
      console.error('Failed to add chat attachment:', err)
      setAttachmentError(
        typeof err === 'string' ? err : err instanceof Error ? err.message : '添加附件失败',
      )
    }
  }, [addAttachments, attachmentsFromPaths, closeProjectMenu, disabled])

  const handleSlashCommandSelect = useCallback(async (command: SlashCommandDefinition) => {
    if (disabled) return

    if (command.id === 'help') {
      setInput('/')
      setActiveSlashToken({ start: 0, end: 1, query: '' })
      setSlashPanelOpen(true)
      setToolPanelOpen(false)
      closeProjectMenu()
      requestAnimationFrame(() => {
        const textarea = textareaRef.current
        if (!textarea) return
        textarea.focus({ preventScroll: true })
        textarea.selectionStart = 1
        textarea.selectionEnd = 1
        updateTextareaHeight()
      })
      return
    }

    removeActiveSlashToken()
    setSlashPanelOpen(false)

    switch (command.id) {
      case 'plan':
        await setAgentPlanMode('plan')
        return
      case 'new':
        setInput('')
        setAttachments([])
        setAttachmentError('')
        requestAnimationFrame(updateTextareaHeight)
        await onNewChat?.()
        return
      case 'compact':
        await onCompactContext?.()
        return
      case 'clear':
        setInput('')
        setAttachments([])
        setAttachmentError('')
        requestAnimationFrame(updateTextareaHeight)
        await onClearChat?.()
        return
      case 'settings':
        onOpenSettings?.()
        return
      case 'tools':
        if (onOpenTools) {
          onOpenTools()
        } else {
          setToolPanelOpen(true)
          closeProjectMenu()
        }
        return
      case 'attach':
        await openAttachmentPicker()
        return
    }
  }, [
    disabled,
    onClearChat,
    onCompactContext,
    onNewChat,
    onOpenSettings,
    onOpenTools,
    openAttachmentPicker,
    removeActiveSlashToken,
    setAgentPlanMode,
    closeProjectMenu,
    updateTextareaHeight,
  ])

  const handleSend = () => {
    const trimmed = input.trim()
    if ((!trimmed && attachments.length === 0) || disabled || sendDisabledReason) return
    onSend(trimmed, attachments)
    setInput('')
    setAttachments([])
    setAttachmentError('')
    setToolPanelOpen(false)
    closeProjectMenu()
    setSlashPanelOpen(false)
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto'
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return

    if (e.key === 'Tab' && e.shiftKey && onAgentPlanModeChange && !disabled) {
      e.preventDefault()
      void toggleAgentPlanMode()
      return
    }

    if (slashPanelOpen) {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        if (filteredSlashCommands.length > 0) {
          setSlashSelectedIndex((index) => (index + 1) % filteredSlashCommands.length)
        }
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        if (filteredSlashCommands.length > 0) {
          setSlashSelectedIndex((index) => (
            index - 1 + filteredSlashCommands.length
          ) % filteredSlashCommands.length)
        }
        return
      }
      if (e.key === 'Tab') {
        e.preventDefault()
        if (selectedSlashCommand) {
          completeActiveSlashToken(selectedSlashCommand)
        }
        return
      }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        if (selectedSlashCommand) {
          void handleSlashCommandSelect(selectedSlashCommand)
        }
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setSlashPanelOpen(false)
        return
      }
    }

    if (e.key !== 'Enter' || e.shiftKey) return
    e.preventDefault()
    handleSend()
  }

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const nextValue = e.target.value
    setInput(nextValue)
    const el = e.target
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`
    syncSlashToken(nextValue, el.selectionStart)
  }

  const handleSelect = (e: React.SyntheticEvent<HTMLTextAreaElement>) => {
    const el = e.currentTarget
    syncSlashToken(el.value, el.selectionStart)
  }

  const handlePaste = async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    if (disabled || !isTauriRuntime()) return

    const attachableClipboardFiles = Array.from(e.clipboardData.files).filter(isAttachableClipboardFile)
    const textarea = textareaRef.current
    const clipText = e.clipboardData.getData('text/plain')
    const selectionStart = textarea?.selectionStart ?? input.length
    const selectionEnd = textarea?.selectionEnd ?? input.length
    const valueBeforePaste = textarea?.value ?? input

    // 剪贴板里已有 File 对象时可同步拦截；系统文件路径只能异步读取，后面再精确撤销文件名文本。
    if (attachableClipboardFiles.length > 0) {
      e.preventDefault()
    }

    const nativePaths: string[] = []
    try {
      const native = await api.chatReadClipboardFiles()
      if (native.success && native.files?.length) {
        nativePaths.push(...native.files.map((file) => file.path))
      }
    } catch (err) {
      console.error('Failed to read clipboard files:', err)
    }

    const hasNativeFiles = nativePaths.length > 0
    const hasClipboardFiles = attachableClipboardFiles.length > 0

    // 纯文字粘贴：不拦截，交给浏览器默认处理
    if (!hasNativeFiles && !hasClipboardFiles) return

    if (hasNativeFiles && textarea) {
      // 等浏览器默认粘贴与 React onChange 完成后，只在内容完全等于“插入了文件名”时撤销。
      window.setTimeout(() => {
        undoAccidentalFilenamePaste(
          textarea,
          valueBeforePaste,
          clipText,
          selectionStart,
          selectionEnd,
          setInput,
        )
      }, 0)
    }

    setAttachmentError('')

    try {
      const pastedAttachments: PendingAttachment[] = []

      if (hasNativeFiles) {
        pastedAttachments.push(...attachmentsFromPaths(nativePaths))
      } else for (const [index, file] of attachableClipboardFiles.entries()) {
        const ext = file.name.split('.').pop()?.toLowerCase() ?? ''

        if (file.type.startsWith('image/') || IMAGE_EXTENSIONS.includes(ext)) {
          const imageExt = file.type.startsWith('image/')
            ? imageExtensionForMime(file.type)
            : ext
          const name = file.name || `pasted-image-${Date.now()}-${index + 1}.${imageExt}`
          const dataBase64 = await readFileAsBase64(file)
          const result = await api.chatSavePastedImage(
            name,
            file.type || `image/${imageExt}`,
            dataBase64,
          )
          if (!result.success || !result.path || !result.name) {
            throw new Error(result.error || '粘贴图片失败')
          }
          pastedAttachments.push({
            id: `pending-att-${crypto.randomUUID()}`,
            type: 'image',
            name: result.name,
            path: result.path,
          })
          continue
        }

        if (file.size <= 0) continue

        const name = file.name || `pasted-file-${Date.now()}-${index + 1}.${ext}`
        const dataBase64 = await readFileAsBase64(file)
        const result = await api.chatSavePastedAttachment(name, dataBase64)
        if (!result.success || !result.path || !result.name) {
          throw new Error(result.error || '粘贴附件失败')
        }
        pastedAttachments.push({
          id: `pending-att-${crypto.randomUUID()}`,
          type: 'file',
          name: result.name,
          path: result.path,
        })
      }

      if (pastedAttachments.length === 0) {
        setAttachmentError('未识别到可添加的文件')
        return
      }

      addAttachments(pastedAttachments)
    } catch (err) {
      console.error('Failed to paste chat attachment:', err)
      setAttachmentError(
        typeof err === 'string' ? err : err instanceof Error ? err.message : '粘贴附件失败',
      )
    }
  }

  const removeAttachment = (id: string) => {
    setAttachments((prev) => prev.filter((attachment) => attachment.id !== id))
    setAttachmentError('')
  }

  useEffect(() => {
    if (!autoFocus || disabled) return
    requestAnimationFrame(() => {
      if (shouldComposerAutoFocus(document.activeElement)) {
        textareaRef.current?.focus({ preventScroll: true })
      }
    })
  }, [autoFocus, disabled])

  useEffect(() => {
    if (!autoFocus || !isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined

    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused || cancelled) return
      requestAnimationFrame(() => {
        if (!cancelled && !disabled && shouldComposerAutoFocus(document.activeElement)) {
          textareaRef.current?.focus({ preventScroll: true })
        }
      })
    }).then((handler) => {
      if (cancelled) {
        handler()
      } else {
        unlisten = handler
      }
    }).catch((err) => {
      console.error('Failed to listen for chat input focus changes:', err)
    })

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [autoFocus, disabled])

  useEffect(() => {
    if (!toolPanelOpen) return
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setToolPanelOpen(false)
      }
    }
    window.addEventListener('keydown', handleEscape)
    return () => window.removeEventListener('keydown', handleEscape)
  }, [toolPanelOpen])

  useEffect(() => {
    if (!projectMenuOpen) return
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closeProjectMenu()
      }
    }
    window.addEventListener('keydown', handleEscape)
    return () => window.removeEventListener('keydown', handleEscape)
  }, [closeProjectMenu, projectMenuOpen])

  useEffect(() => {
    if (!slashPanelOpen) return
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target
      if (!(target instanceof Element)) return
      if (target.closest('[data-chat-slash-panel="true"]')) return
      if (target.closest('[data-chat-composer="true"]')) return
      setSlashPanelOpen(false)
    }
    window.addEventListener('pointerdown', handlePointerDown)
    return () => window.removeEventListener('pointerdown', handlePointerDown)
  }, [slashPanelOpen])

  useLayoutEffect(() => {
    if (!slashPanelOpen) return

    const updateSlashPanelLeft = () => {
      const inner = innerRef.current
      const textarea = textareaRef.current
      if (!inner || !textarea) return

      const innerRect = inner.getBoundingClientRect()
      const textareaRect = textarea.getBoundingClientRect()
      setSlashPanelLeft(Math.max(0, Math.round(textareaRect.left - innerRect.left)))
    }

    updateSlashPanelLeft()
    window.addEventListener('resize', updateSlashPanelLeft)

    const resizeObserver = typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver(updateSlashPanelLeft)
    if (resizeObserver) {
      if (innerRef.current) resizeObserver.observe(innerRef.current)
      if (textareaRef.current) resizeObserver.observe(textareaRef.current)
    }

    return () => {
      window.removeEventListener('resize', updateSlashPanelLeft)
      resizeObserver?.disconnect()
    }
  }, [slashPanelOpen])

  useEffect(() => {
    if (!disabled) return
    setToolPanelOpen(false)
    closeProjectMenu()
    setSlashPanelOpen(false)
  }, [closeProjectMenu, disabled])

  useEffect(() => {
    setSlashSelectedIndex(0)
  }, [activeSlashToken?.query])

  useEffect(() => {
    if (slashSelectedIndex < filteredSlashCommands.length) return
    setSlashSelectedIndex(Math.max(filteredSlashCommands.length - 1, 0))
  }, [filteredSlashCommands.length, slashSelectedIndex])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined

    getCurrentWebview().onDragDropEvent((event) => {
      if (cancelled || disabled) return

      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        setDragActive(true)
        setAttachmentError('')
        return
      }

      if (event.payload.type === 'leave') {
        setDragActive(false)
        return
      }

      if (event.payload.type === 'drop') {
        setDragActive(false)
        addAttachments(attachmentsFromPaths(event.payload.paths))
      }
    }).then((handler) => {
      if (cancelled) {
        handler()
      } else {
        unlisten = handler
      }
    }).catch((err) => {
      console.error('Failed to listen for chat attachment drops:', err)
    })

    return () => {
      cancelled = true
      setDragActive(false)
      unlisten?.()
    }
  }, [addAttachments, attachmentsFromPaths, disabled])

  const canSend = (Boolean(input.trim()) || attachments.length > 0)
    && !slashPanelOpen
    && !disabled
    && !sendDisabledReason

  const wrapperClass =
    layout === 'inline'
      ? 'w-full'
      : 'chat-composer-footer shrink-0 px-6 pb-8 pt-2'

  const innerClass = layout === 'inline' ? 'w-full' : 'mx-auto w-full max-w-3xl'
  const slashPanelPlacementClass = layout === 'inline'
    ? 'top-full mt-1'
    : 'bottom-full mb-1'
  const slashPanelOrigin = layout === 'inline' ? 'top left' : 'bottom left'
  const projectPanelPlacementClass = layout === 'inline'
    ? 'top-full mt-1.5'
    : 'bottom-full mb-1.5'
  const projectPanelOrigin = layout === 'inline' ? 'top left' : 'bottom left'
  const externalMcpTools = enabledTools.filter(isExternalMcpTool)
  const hasToolProblem = Boolean(toolsDisabledReason || toolStatusHint || sendDisabledReason)
  const showMcpSection = externalMcpTools.length > 0 || Boolean(toolsDisabledReason)
  const mcpStatusLine = toolsDisabledReason
    || (externalMcpTools.length > 0 ? `MCP ${externalMcpTools.length}` : '')
  const approvalOption = approvalPolicyOption(approvalPolicy)

  return (
    <div className={wrapperClass}>
      <div ref={innerRef} className={`relative ${innerClass}`}>
        {toolPanelOpen && (
          <>
            <div className="fixed inset-0 z-30" onClick={() => setToolPanelOpen(false)} aria-hidden />
            <div
              className="chat-motion-popover absolute bottom-full left-10 z-40 mb-2 w-[min(320px,calc(100vw-32px))] overflow-hidden rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] shadow-[0_10px_28px_rgba(0,0,0,0.14)] dark:border-neutral-700 dark:bg-neutral-900"
              style={{ ['--chat-popover-origin' as string]: 'bottom left' }}
              data-tauri-drag-region="false"
            >
              <div className="space-y-1.5 px-3 py-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">Skill</span>
                  {onOpenSkillSettings && (
                    <button
                      type="button"
                      onClick={() => {
                        setToolPanelOpen(false)
                        onOpenSkillSettings()
                      }}
                      className="rounded-md px-1.5 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                    >
                      管理
                    </button>
                  )}
                </div>
                <div className="text-[11px] leading-4 text-neutral-600 dark:text-neutral-300">
                  <span className="text-neutral-500 dark:text-neutral-400">
                    已启用 {enabledSkills.length} 个
                  </span>
                  {enabledSkills.length > 0 && (
                    <>
                      <span className="text-neutral-300 dark:text-neutral-600"> · </span>
                      <span className="text-neutral-700 dark:text-neutral-200">
                        {enabledSkills.map((skill) => skill.name).join('、')}
                      </span>
                    </>
                  )}
                </div>

                {onApprovalPolicyChange && (
                  <div className="border-t border-neutral-200/80 pt-1.5 dark:border-neutral-800">
                    <div className="mb-1.5 flex items-center justify-between gap-2 text-[11px] text-neutral-500 dark:text-neutral-400">
                      <span className="inline-flex items-center gap-1">
                        <ShieldAlert size={13} strokeWidth={1.8} />
                        审批
                      </span>
                      <span className={approvalOption.value === 'auto' ? 'font-semibold text-[#e9531f] dark:text-[#ff9a71]' : ''}>
                        {approvalOption.label}
                      </span>
                    </div>
                    <div className="grid grid-cols-3 gap-1">
                      {APPROVAL_POLICY_OPTIONS.map((option) => {
                        const selected = option.value === approvalOption.value
                        return (
                          <button
                            key={option.value}
                            type="button"
                            onClick={() => void onApprovalPolicyChange(option.value)}
                            className={`rounded-md px-1.5 py-1 text-[11px] font-medium transition-colors ${
                              selected
                                ? option.value === 'auto'
                                  ? 'bg-[#fff1eb] text-[#e9531f] dark:bg-[#f26b2d]/15 dark:text-[#ff9a71]'
                                  : 'bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100'
                                : 'text-neutral-500 hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100'
                            }`}
                          >
                            {option.label}
                          </button>
                        )
                      })}
                    </div>
                  </div>
                )}

                {showMcpSection && mcpStatusLine && (
                  <div className="border-t border-neutral-200/80 pt-1.5 text-[11px] text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
                    {mcpStatusLine}
                  </div>
                )}

                {(sendDisabledReason || toolStatusHint) && (
                  <p className="rounded-md bg-amber-50 px-2 py-1 text-[11px] leading-4 text-amber-700 dark:bg-amber-400/10 dark:text-amber-200">
                    {sendDisabledReason || toolStatusHint}
                  </p>
                )}
              </div>
            </div>
          </>
        )}
        {slashPanelOpen && (
          <div
            className={`chat-motion-popover absolute z-40 overflow-hidden rounded-lg border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] p-0.5 font-sans shadow-[0_6px_18px_-16px_rgba(0,0,0,0.2),0_1px_4px_rgba(0,0,0,0.05)] dark:border-neutral-700 dark:bg-neutral-900 ${slashPanelPlacementClass}`}
            style={{
              ['--chat-popover-origin' as string]: slashPanelOrigin,
              ['--chat-popover-start-y' as string]: '0px',
              left: slashPanelLeft,
              width: `calc(100% - ${slashPanelLeft}px)`,
            }}
            data-chat-slash-panel="true"
            data-tauri-drag-region="false"
          >
            <div className="max-h-[min(184px,34vh)] overflow-y-auto">
              {filteredSlashCommands.length > 0 ? (
                filteredSlashCommands.map((command, index) => {
                  const Icon = slashCommandIcon(command.id)
                  const selected = index === slashSelectedIndex
                  return (
                    <button
                      key={command.id}
                      type="button"
                      aria-selected={selected}
                      onMouseEnter={() => setSlashSelectedIndex(index)}
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={() => void handleSlashCommandSelect(command)}
                      className={`flex h-[26px] w-full min-w-0 items-center gap-1.5 rounded-md px-2 text-left transition-colors ${
                        selected
                          ? 'bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-50'
                          : 'text-neutral-700 hover:bg-neutral-50 dark:text-neutral-200 dark:hover:bg-neutral-800/70'
                      }`}
                    >
                      <Icon
                        size={13}
                        strokeWidth={1.8}
                        className="shrink-0 text-neutral-600 dark:text-neutral-300"
                      />
                      <span className="min-w-0 flex-1 truncate text-[12px] leading-none">
                        <span className="font-semibold">{command.title}</span>
                        <span className="ml-1.5 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                          {command.description}
                        </span>
                      </span>
                    </button>
                  )
                })
              ) : (
                <div className="flex h-[26px] items-center px-2 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
                  No matching command
                </div>
              )}
            </div>
          </div>
        )}
        {projectMenuOpen && projectEntryEnabled && (
          <>
            <div
              className="fixed inset-0 z-30"
              onClick={closeProjectMenu}
              aria-hidden
            />
            <div
              className={`chat-motion-popover absolute left-0 z-40 w-[min(240px,calc(100vw-32px))] overflow-visible rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] p-1 shadow-[0_10px_24px_rgba(0,0,0,0.12)] dark:border-neutral-700 dark:bg-neutral-900 ${projectPanelPlacementClass}`}
              style={{ ['--chat-popover-origin' as string]: projectPanelOrigin }}
              data-tauri-drag-region="false"
            >
              <div className="flex h-7 items-center gap-1.5 rounded-md px-2 text-neutral-500 dark:text-neutral-400">
                <Search size={14} strokeWidth={1.8} className="shrink-0" />
                <input
                  value={projectSearchQuery}
                  onChange={(event) => setProjectSearchQuery(event.target.value)}
                  placeholder="搜索项目"
                  className="min-w-0 flex-1 border-0 bg-transparent text-[12px] font-semibold text-neutral-800 outline-none placeholder:text-neutral-400 dark:text-neutral-100 dark:placeholder:text-neutral-500"
                />
              </div>

              <div className="mt-0.5 max-h-48 overflow-y-auto">
                {projectOptionsLoading ? (
                  <div className="px-2 py-2.5 text-[12px] text-neutral-400 dark:text-neutral-500">
                    正在加载项目…
                  </div>
                ) : projectOptionsError ? (
                  <div className="px-2 py-2 text-[12px] text-red-500 dark:text-red-400">
                    {projectOptionsError}
                  </div>
                ) : visibleProjectOptions.length > 0 ? (
                  <div className="py-1">
                    {visibleProjectOptions.map((project) => {
                      const active = selectedProject?.id === project.id
                      const pathLabel = projectPathLabel(project)
                      return (
                        <button
                          key={project.id}
                          type="button"
                          onClick={() => void selectProject(project)}
                          className={`flex min-h-[34px] w-full min-w-0 items-center gap-1.5 rounded-md px-2 text-left transition-colors ${
                            active
                              ? 'bg-neutral-100 text-neutral-950 dark:bg-neutral-800 dark:text-neutral-50'
                              : 'text-neutral-800 hover:bg-neutral-100 dark:text-neutral-200 dark:hover:bg-neutral-800'
                          }`}
                        >
                          <Folder size={14} strokeWidth={1.75} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-[12px] font-semibold">{project.name}</span>
                            {pathLabel && (
                              <span className="block truncate text-[10px] font-medium text-neutral-400 dark:text-neutral-500">
                                {pathLabel}
                              </span>
                            )}
                          </span>
                          {active && <Check size={13} strokeWidth={2} className="shrink-0 text-neutral-500 dark:text-neutral-300" />}
                        </button>
                      )
                    })}
                  </div>
                ) : (
                  <div className="px-2 py-2.5 text-[12px] leading-5 text-neutral-400 dark:text-neutral-500">
                    {projectSearchQuery.trim() ? '没有匹配的项目' : '还没有最近项目'}
                  </div>
                )}
              </div>

              <div className="mt-0.5 border-t border-neutral-200/80 pt-0.5 dark:border-neutral-800">
                {selectedProject && (
                  <button
                    type="button"
                    onClick={() => void selectProject(null)}
                    className="flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-[12px] font-semibold text-neutral-500 transition-colors hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                  >
                    <Folder size={14} strokeWidth={1.75} className="shrink-0" />
                    <span className="min-w-0 flex-1 truncate">退出项目工作</span>
                  </button>
                )}
                <div className="relative">
                  <button
                    type="button"
                    onClick={() => setProjectCreateMenuOpen((open) => !open)}
                    disabled={projectCreating}
                    className={`flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-[12px] font-semibold transition-colors disabled:cursor-default disabled:opacity-50 ${
                      projectCreateMenuOpen
                        ? 'bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100'
                        : 'text-neutral-800 hover:bg-neutral-100 dark:text-neutral-100 dark:hover:bg-neutral-800'
                    }`}
                    aria-haspopup="menu"
                    aria-expanded={projectCreateMenuOpen}
                  >
                    <FolderPlus size={14} strokeWidth={1.75} className="shrink-0 text-neutral-600 dark:text-neutral-300" />
                    <span className="min-w-0 flex-1 truncate">
                      {projectCreating ? '正在添加…' : '添加新项目'}
                    </span>
                    <ChevronRight size={13} strokeWidth={1.9} className="shrink-0 text-neutral-400" />
                  </button>
                  {projectCreateMenuOpen && (
                    <div
                      className="absolute left-0 top-full z-50 mt-1 w-32 rounded-lg border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] p-1 shadow-[0_10px_24px_rgba(0,0,0,0.12)] dark:border-neutral-700 dark:bg-neutral-900 sm:bottom-0 sm:left-full sm:top-auto sm:mt-0 sm:ml-1"
                      role="menu"
                    >
                      <button
                        type="button"
                        onClick={() => void createBlankProject()}
                        disabled={projectCreating}
                        className="flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-[12px] font-semibold text-neutral-800 transition-colors hover:bg-neutral-100 disabled:cursor-default disabled:opacity-50 dark:text-neutral-100 dark:hover:bg-neutral-800"
                      >
                        <Plus size={14} strokeWidth={1.8} className="shrink-0 text-neutral-600 dark:text-neutral-300" />
                        <span className="min-w-0 flex-1 truncate">新建空白项目</span>
                      </button>
                      <button
                        type="button"
                        onClick={() => void createProjectFromFolder()}
                        disabled={projectCreating}
                        className="flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-[12px] font-semibold text-neutral-800 transition-colors hover:bg-neutral-100 disabled:cursor-default disabled:opacity-50 dark:text-neutral-100 dark:hover:bg-neutral-800"
                      >
                        <Folder size={14} strokeWidth={1.75} className="shrink-0 text-neutral-600 dark:text-neutral-300" />
                        <span className="min-w-0 flex-1 truncate">使用现有文件夹</span>
                      </button>
                    </div>
                  )}
                </div>
              </div>
            </div>
          </>
        )}
        <div
          data-chat-composer="true"
          className={`chat-composer-shell relative z-10 rounded-[28px] border px-3 py-2.5 transition-[box-shadow,border-color] duration-200 ${
            dragActive
              ? 'border-[#e8a090] shadow-[0_2px_12px_rgba(0,0,0,0.06)] ring-2 ring-[#e8a090]/25 dark:border-[#e8a090] dark:shadow-none'
              : agentPlanActive
                ? 'border-emerald-500 shadow-[0_1px_2px_rgba(0,0,0,0.04),0_12px_32px_-14px_rgba(0,0,0,0.14)] focus-within:border-emerald-500 focus-within:shadow-[0_1px_3px_rgba(0,0,0,0.05),0_18px_44px_-16px_rgba(16,185,129,0.22)] dark:border-emerald-400 dark:shadow-none dark:focus-within:border-emerald-400'
                : 'border-neutral-200/80 shadow-[0_1px_2px_rgba(0,0,0,0.04),0_12px_32px_-14px_rgba(0,0,0,0.14)] focus-within:border-neutral-300 focus-within:shadow-[0_1px_3px_rgba(0,0,0,0.05),0_18px_44px_-16px_rgba(0,0,0,0.20)] dark:border-neutral-700 dark:shadow-none dark:focus-within:border-neutral-600'
          }`}
        >
          {dragActive && (
            <div className="chat-motion-fade-up mb-2 rounded-2xl border border-dashed border-[#e8a090]/70 bg-[#e8a090]/10 px-3 py-2 text-center text-[13px] font-medium text-[#a35f51] dark:text-[#f1b4a7]">
              松开即可添加附件
            </div>
          )}
          {attachments.length > 0 && (
            <div className="chat-motion-fade-up mb-2 px-1">
              <ChatAttachments
                attachments={attachments}
                variant="composer"
                onRemove={disabled ? undefined : removeAttachment}
              />
            </div>
          )}
          {attachmentError && (
            <div className="chat-motion-fade-up mb-2 px-1 text-[12px] text-red-500 dark:text-red-400">
              {attachmentError}
            </div>
          )}
          {(sendDisabledReason || toolStatusHint) && !attachmentError && (
            <div className="chat-motion-fade-up mb-2 px-1 text-[12px] text-amber-600 dark:text-amber-300">
              {sendDisabledReason || toolStatusHint}
            </div>
          )}
          <div className="flex items-end gap-2">
            <button
              type="button"
              onClick={() => void openAttachmentPicker()}
              disabled={disabled}
              tabIndex={-1}
              className="mb-0.5 shrink-0 rounded-full p-2 text-neutral-500 transition-colors hover:bg-neutral-100 disabled:opacity-40 dark:hover:bg-neutral-800"
              title="添加附件"
              aria-label="添加附件"
            >
              <Plus size={20} strokeWidth={1.75} />
            </button>

            {onOpenSettings && (
              <button
                type="button"
                onClick={() => {
                  setSlashPanelOpen(false)
                  closeProjectMenu()
                  setToolPanelOpen((open) => !open)
                }}
                disabled={disabled}
                tabIndex={-1}
                className={`mb-0.5 shrink-0 rounded-full p-2 transition-colors disabled:opacity-40 ${
                  toolPanelOpen || hasToolProblem
                    ? 'bg-neutral-100 text-neutral-800 dark:bg-neutral-800 dark:text-neutral-100'
                    : 'text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800'
                }`}
                title="MCP / Skill"
                aria-label="MCP / Skill"
              >
                <SlidersHorizontal size={18} strokeWidth={1.75} />
              </button>
            )}

            <textarea
              ref={textareaRef}
              value={input}
              onChange={handleInput}
              onPaste={(e) => void handlePaste(e)}
              onKeyDown={handleKeyDown}
              onSelect={handleSelect}
              disabled={disabled}
              placeholder="Ask me anything..."
              rows={1}
              className="mb-0.5 max-h-40 min-h-[28px] flex-1 resize-none border-0 bg-transparent px-1 py-1.5 text-[15px] leading-relaxed text-neutral-900 outline-none placeholder:text-neutral-400 disabled:opacity-50 dark:text-neutral-100"
            />

            {onExecuteAgentPlan && agentPlanText && (
              <button
                type="button"
                onClick={() => void onExecuteAgentPlan()}
                disabled={disabled}
                tabIndex={-1}
                className="mb-0.5 flex h-8 shrink-0 items-center gap-1 rounded-full bg-neutral-900 px-2.5 text-[12px] font-medium text-white transition-colors hover:bg-neutral-700 disabled:bg-neutral-200 disabled:text-neutral-400 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 dark:disabled:bg-neutral-700 dark:disabled:text-neutral-500"
                title="执行当前计划"
                aria-label="执行当前计划"
              >
                <Play size={13} strokeWidth={2.1} fill="currentColor" />
                执行
              </button>
            )}

            {cancelVisible && onCancel ? (
              <button
                type="button"
                onClick={onCancel}
                disabled={cancelling}
                className="chat-motion-fade-up mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-neutral-900 text-white shadow-sm transition-all hover:bg-neutral-700 disabled:bg-neutral-300 disabled:text-neutral-500 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 dark:disabled:bg-neutral-700 dark:disabled:text-neutral-500"
                title={cancelling ? '正在停止' : '停止生成'}
                aria-label={cancelling ? '正在停止' : '停止生成'}
              >
                <Square size={13} strokeWidth={2.4} fill="currentColor" />
              </button>
            ) : (
              <button
                type="button"
                onClick={handleSend}
                disabled={!canSend}
                tabIndex={-1}
                title={sendDisabledReason || (canSend ? '发送' : '输入消息后发送')}
                aria-label={sendDisabledReason || '发送'}
                className={`mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-all ${
                  canSend
                    ? 'chat-motion-soft-pulse bg-[#e8a090] text-white shadow-sm hover:bg-[#df9585]'
                    : 'bg-neutral-200 text-neutral-400 dark:bg-neutral-700 dark:text-neutral-500'
                }`}
              >
                <ArrowUp size={18} strokeWidth={2.25} />
              </button>
            )}
          </div>
        </div>
        {projectEntryEnabled && (
          <div className="relative z-0 -mt-3 flex min-h-[40px] w-full items-end rounded-b-[24px] bg-[color-mix(in_srgb,var(--theme-surface-muted)_72%,transparent)] px-3 pb-2 pt-4 dark:bg-neutral-800/50">
            <button
              type="button"
              onClick={toggleProjectMenu}
              disabled={disabled}
              className={`inline-flex h-[26px] max-w-full items-center gap-1 rounded-full px-2 text-left text-[12px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-300/60 dark:focus-visible:ring-neutral-600 ${
                projectMenuOpen
                  ? 'bg-neutral-200 text-neutral-800 shadow-[inset_0_1px_0_rgba(255,255,255,0.55)] dark:bg-neutral-700 dark:text-neutral-100'
                  : selectedProject
                    ? 'bg-neutral-200/50 text-neutral-700 hover:bg-neutral-200/80 dark:bg-neutral-700/45 dark:text-neutral-200 dark:hover:bg-neutral-700/70'
                    : 'bg-transparent text-neutral-500 hover:bg-neutral-200/70 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-700/55 dark:hover:text-neutral-100'
              } disabled:cursor-default disabled:opacity-50`}
              aria-expanded={projectMenuOpen}
              aria-haspopup="menu"
            >
              {selectedProject ? (
                <Folder size={13} strokeWidth={1.75} className="shrink-0 text-neutral-500 dark:text-neutral-300" />
              ) : (
                <FolderPlus size={13} strokeWidth={1.75} className="shrink-0 text-neutral-500 dark:text-neutral-300" />
              )}
              <span className="min-w-0 truncate">
                {selectedProject ? selectedProject.name : '进入项目工作'}
              </span>
              <ChevronDown
                size={12}
                strokeWidth={2}
                className={`shrink-0 text-neutral-400 transition-transform ${
                  projectMenuOpen ? 'rotate-180' : ''
                }`}
              />
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
