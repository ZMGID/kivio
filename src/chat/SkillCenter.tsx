import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowLeft,
  Box,
  ChevronDown,
  Download,
  ExternalLink,
  FolderOpen,
  Plus,
  RefreshCw,
  Search,
  Sliders,
  Sparkles,
  Trash2,
  X,
} from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'
import { ChatMarkdown } from './ChatMarkdown'
import {
  api,
  defaultNativeTools,
  type ChatToolsConfig,
  type Settings,
  type SkillDetail,
  type SkillMeta,
} from '../api/tauri'
import { getSettingsCached, refreshSettings, saveSettingsCached } from '../api/settingsCache'
import { usesNativeTitlebar } from './platform'
import { Select } from '../settings/components'
import { Button, IconButton } from '../components/Button'
import { SkillStoreBrowser } from './SkillStoreBrowser'

interface SkillCenterProps {
  /** 返回对话视图 */
  onClose: () => void
  /** Skill 启用状态 / 列表变化后通知 Chat 刷新其技能列表 */
  onSkillsChanged?: () => void
}

function defaultChatTools(): ChatToolsConfig {
  return {
    enabled: false,
    servers: [],
    skillScanPaths: [],
    skillAutoMatch: true,
    skillFallbackMode: 'progressive',
    disabledSkillIds: [],
    maxToolRounds: 20,
    toolTimeoutMs: 60_000,
    mcpIdleTimeoutMs: 600_000,
    approvalPolicy: 'readonly_auto_sensitive_confirm',
    nativeTools: defaultNativeTools(),
  }
}

function isBuiltinSkill(skill: SkillMeta): boolean {
  return skill.source === 'builtin'
}

function isPluginSkill(skill: SkillMeta): boolean {
  return skill.source === 'plugin'
}

function skillSourceLabel(skill: SkillMeta): string {
  if (skill.source === 'builtin') return '内置'
  if (skill.source === 'plugin') return '插件'
  if (skill.source === 'external') return '工作区'
  return '个人'
}

function skillMatches(skill: SkillMeta, query: string): boolean {
  if (!query) return true
  return (
    skill.name.toLowerCase().includes(query) ||
    (skill.description ?? '').toLowerCase().includes(query)
  )
}

/** 自带样式的开关：明暗对比清晰，不依赖设置面板的 CSS 变量作用域 */
function Switch({
  checked,
  onChange,
  ariaLabel,
}: {
  checked: boolean
  onChange: (value: boolean) => void
  ariaLabel?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      onClick={() => onChange(!checked)}
      data-tauri-drag-region="false"
      className={`relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full transition-colors focus:outline-none ${
        checked
          ? 'bg-emerald-500 hover:bg-emerald-600'
          : 'bg-neutral-300 hover:bg-neutral-400 dark:bg-neutral-600 dark:hover:bg-neutral-500'
      }`}
    >
      <span
        className={`inline-block size-[18px] rounded-full bg-white shadow-sm transition-transform ${
          checked ? 'translate-x-[18px]' : 'translate-x-0.5'
        }`}
      />
    </button>
  )
}

function SkillRow({
  skill,
  enabled,
  onToggleEnabled,
  onPreview,
  manageLocked = false,
}: {
  skill: SkillMeta
  enabled: boolean
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
  /** 插件附属：开关在插件页，此处只展示 */
  manageLocked?: boolean
}) {
  return (
    <div
      className={`flex min-w-0 items-center gap-3 px-4 py-3 transition-colors hover:bg-neutral-50 dark:hover:bg-neutral-900/50 ${
        enabled ? '' : 'opacity-60'
      }`}
    >
      <button
        type="button"
        onClick={() => onPreview(skill.id)}
        className="flex min-w-0 flex-1 items-center gap-3 text-left"
        data-tauri-drag-region="false"
        title="查看完整内容"
      >
        <span className="grid size-9 shrink-0 place-items-center rounded-md border border-neutral-200 bg-white text-neutral-400 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-500">
          <Box size={16} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[14px] font-semibold text-neutral-950 dark:text-neutral-50">
            {skill.name}
          </span>
          <span className="mt-0.5 block truncate text-[12px] text-neutral-500 dark:text-neutral-400">
            {skill.description || '未设置描述'}
          </span>
        </span>
      </button>
      <span className="shrink-0 text-[13px] text-neutral-400 dark:text-neutral-500">{skillSourceLabel(skill)}</span>
      {manageLocked ? (
        <span
          className="shrink-0 text-[12px] text-neutral-400 dark:text-neutral-500"
          title="请在 扩展 → 插件 中启用/关闭整包插件"
        >
          {enabled ? '插件已启用' : '随插件关闭'}
        </span>
      ) : (
        <Switch checked={enabled} onChange={(next) => onToggleEnabled(skill.id, next)} ariaLabel={`启用 ${skill.name}`} />
      )}
    </div>
  )
}

function SkillSection({
  title,
  note,
  emptyText,
  skills,
  disabledSkillIds,
  onToggleEnabled,
  onPreview,
  collapsible = false,
  defaultCollapsed = false,
  manageLocked = false,
}: {
  title: string
  note?: string
  emptyText: string
  skills: SkillMeta[]
  disabledSkillIds: string[]
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
  collapsible?: boolean
  defaultCollapsed?: boolean
  manageLocked?: boolean
}) {
  const [collapsed, setCollapsed] = useState(collapsible && defaultCollapsed)
  const enabledCount = skills.filter((skill) => !disabledSkillIds.includes(skill.id)).length
  return (
    <section className="space-y-2.5">
      <div
        className={`flex min-w-0 items-center gap-3 px-1 ${collapsible ? 'cursor-pointer select-none' : ''}`}
        onClick={collapsible ? () => setCollapsed((v) => !v) : undefined}
      >
        {collapsible && (
          <ChevronDown
            size={16}
            className={`shrink-0 text-neutral-400 transition-transform duration-[var(--kv-dur-fast)] ease-[var(--kv-ease-standard)] ${collapsed ? '-rotate-90' : ''}`}
          />
        )}
        <h3 className="text-[15px] font-semibold text-neutral-700 dark:text-neutral-200">{title}</h3>
        <span className="text-[14px] font-medium text-neutral-400">{skills.length}</span>
        {collapsed && skills.length > 0 && (
          <span className="text-[12.5px] text-neutral-400">已启用 {enabledCount}</span>
        )}
        {note && <span className="ml-auto truncate text-[12.5px] text-neutral-400">{note}</span>}
      </div>
      {collapsed ? null : skills.length === 0 ? (
        <div className="grid min-h-[72px] place-items-center rounded-md border border-dashed border-neutral-200 text-[13px] text-neutral-400 dark:border-neutral-800">
          {emptyText}
        </div>
      ) : (
        <div className="overflow-hidden rounded-md border border-neutral-200 bg-white shadow-sm dark:border-neutral-800 dark:bg-neutral-950/40 [&>*+*]:border-t [&>*+*]:border-neutral-100 dark:[&>*+*]:border-neutral-800/70">
          {skills.map((skill) => (
            <SkillRow
              key={skill.id}
              skill={skill}
              enabled={manageLocked ? true : !disabledSkillIds.includes(skill.id)}
              onToggleEnabled={onToggleEnabled}
              onPreview={onPreview}
              manageLocked={manageLocked}
            />
          ))}
        </div>
      )}
    </section>
  )
}

function SkillUrlImport({ onInstalled }: { onInstalled: () => void }) {
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [done, setDone] = useState('')
  const install = useCallback(async () => {
    const value = url.trim()
    if (!value) return
    setBusy(true)
    setError('')
    setDone('')
    try {
      const result = await api.chatSkillsInstallFromUrl(value)
      if (!result.success) throw new Error(result.error || '安装失败')
      setDone('已安装')
      setUrl('')
      onInstalled()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }, [url, onInstalled])
  return (
    <div className="rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
      <div className="mb-1.5 text-[13px] font-medium text-neutral-800 dark:text-neutral-100">从 URL 安装</div>
      <p className="mb-2 text-[12px] text-neutral-500 dark:text-neutral-400">
        粘贴 GitHub 仓库地址或直链 .zip（须含 SKILL.md）。仓库内多个技能只安装第一个。
      </p>
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="https://github.com/owner/repo"
          className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2.5 font-mono text-[12.5px] text-neutral-800 outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
          data-tauri-drag-region="false"
        />
        <Button onClick={() => void install()} disabled={busy || !url.trim()} data-tauri-drag-region="false">
          {busy ? '安装中…' : '安装'}
        </Button>
      </div>
      {error && <div className="mt-2 text-[12px] text-red-600 dark:text-red-400">{error}</div>}
      {done && <div className="mt-2 text-[12px] text-emerald-600 dark:text-emerald-400">{done}</div>}
    </div>
  )
}

export function SkillCenter({ onClose, onSkillsChanged }: SkillCenterProps) {
  const [settings, setSettings] = useState<Settings | null>(null)
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [skillsLoading, setSkillsLoading] = useState(false)
  const [skillError, setSkillError] = useState('')
  const [query, setQuery] = useState('')
  const [view, setView] = useState<'installed' | 'store' | 'import' | 'advanced'>('installed')
  const [selectedSkillPreview, setSelectedSkillPreview] = useState<SkillDetail | null>(null)

  const settingsRef = useRef<Settings | null>(null)
  const saveTimer = useRef<number | null>(null)

  const chatTools = settings?.chatTools ?? defaultChatTools()
  const disabledSkillIds = chatTools.disabledSkillIds ?? []

  const refreshChatSkills = useCallback(async (scanPaths?: string[]) => {
    setSkillsLoading(true)
    setSkillError('')
    try {
      const result = await api.chatSkillsList(scanPaths ?? settingsRef.current?.chatTools?.skillScanPaths)
      if (result.success) {
        setSkills(result.skills)
        if (result.error) setSkillError(result.error)
      } else {
        setSkillError(result.error || 'Skill 列表加载失败')
      }
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    } finally {
      setSkillsLoading(false)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const loaded = await getSettingsCached()
        if (cancelled) return
        settingsRef.current = loaded
        setSettings(loaded)
        await refreshChatSkills(loaded.chatTools?.skillScanPaths)
      } catch (err) {
        if (!cancelled) setSkillError(err instanceof Error ? err.message : String(err))
      }
    })()
    return () => {
      cancelled = true
      if (saveTimer.current) window.clearTimeout(saveTimer.current)
    }
  }, [refreshChatSkills])

  const flushSave = useCallback(async (next: Settings) => {
    try {
      // 整体保存前现读后端最新态，仅把 servers 取自 fresh：servers[].auth/headers 会被后端
      // OAuth 刷新独立改写（mcp/manager.rs），若用本地快照整体保存会覆盖回旧 token。
      // 其余 chatTools 字段（技能开关/扫描路径等）是本任务编辑值，仍以本地 next 为准。
      const fresh = await refreshSettings()
      const merged: Settings = {
        ...next,
        chatTools: {
          ...(next.chatTools ?? defaultChatTools()),
          servers: fresh.chatTools?.servers ?? next.chatTools?.servers ?? [],
        },
      }
      const saved = await saveSettingsCached(merged)
      settingsRef.current = saved
      onSkillsChanged?.()
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [onSkillsChanged])

  // 更新 chatTools：本地立即生效，再持久化（文本类编辑防抖，开关/下拉立即保存）
  const persistChatTools = useCallback((updates: Partial<ChatToolsConfig>, debounce = false) => {
    setSettings((prev) => {
      if (!prev) return prev
      const next: Settings = {
        ...prev,
        chatTools: { ...(prev.chatTools ?? defaultChatTools()), ...updates },
      }
      settingsRef.current = next
      if (saveTimer.current) {
        window.clearTimeout(saveTimer.current)
        saveTimer.current = null
      }
      if (debounce) {
        saveTimer.current = window.setTimeout(() => {
          saveTimer.current = null
          void flushSave(next)
        }, 500)
      } else {
        void flushSave(next)
      }
      return next
    })
  }, [flushSave])

  const handleToggleSkillEnabled = useCallback((skillId: string, enabled: boolean) => {
    const disabled = settingsRef.current?.chatTools?.disabledSkillIds ?? []
    const next = enabled
      ? disabled.filter((id) => id !== skillId)
      : disabled.includes(skillId)
        ? disabled
        : [...disabled, skillId]
    persistChatTools({ disabledSkillIds: next })
  }, [persistChatTools])

  const handlePreviewSkill = useCallback(async (skillId: string) => {
    setSkillError('')
    try {
      const result = await api.chatSkillsRead(skillId)
      if (result.success && result.skill) {
        setSelectedSkillPreview(result.skill)
      } else {
        setSkillError(result.error || '读取 Skill 失败')
      }
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [])

  const handleImportSkill = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false })
      if (typeof selected !== 'string') return
      const result = await api.chatSkillsImport(selected)
      if (!result.success) {
        setSkillError(result.error || '导入 Skill 失败')
        return
      }
      await refreshChatSkills()
      onSkillsChanged?.()
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [onSkillsChanged, refreshChatSkills])

  const handleImportSkillZip = useCallback(async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{ name: 'Skill Zip', extensions: ['zip'] }],
      })
      if (typeof selected !== 'string') return
      const result = await api.chatSkillsImport(selected)
      if (!result.success) {
        setSkillError(result.error || '导入 Skill 失败')
        return
      }
      await refreshChatSkills()
      onSkillsChanged?.()
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [onSkillsChanged, refreshChatSkills])

  const handleOpenSkillFolder = useCallback(async () => {
    setSkillError('')
    try {
      const result = await api.chatSkillsOpenFolder()
      if (!result.success) {
        setSkillError(result.error || '打开 Skill 文件夹失败')
      }
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [])

  const normalizedQuery = query.trim().toLowerCase()
  const builtinSkills = useMemo(
    () => skills.filter((skill) => isBuiltinSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )
  const pluginSkills = useMemo(
    () => skills.filter((skill) => isPluginSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )
  const userSkills = useMemo(
    () =>
      skills.filter(
        (skill) =>
          !isBuiltinSkill(skill) && !isPluginSkill(skill) && skillMatches(skill, normalizedQuery),
      ),
    [skills, normalizedQuery],
  )

  return (
    <div className="assistant-center-root flex h-full min-h-0 flex-col text-neutral-900 dark:text-neutral-100">
      {/* 顶栏：与聊天主区同底色、无分隔；可拖拽，右侧避开窗口按钮 */}
      <div
        className={`flex h-[52px] shrink-0 items-center gap-2 px-3 ${
          !usesNativeTitlebar ? 'chat-win-titlebar-safe' : ''
        }`}
        data-tauri-drag-region
      >
        <Button
          variant="ghost"
          size="sm"
          className="shrink-0"
          onClick={onClose}
          data-tauri-drag-region="false"
        >
          <ArrowLeft size={15} />
          返回聊天
        </Button>
        <div className="h-full min-w-5 flex-1" data-tauri-drag-region />
      </div>

      {/* 内容区：直接坐在白底上，与聊天主区无缝 */}
      <main className="custom-scrollbar min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-[1040px] px-9 pb-10 pt-7">
            {/* 头部：标题 + 副标题 + 图标动作 */}
            <div className="border-b border-neutral-200 pb-5 dark:border-neutral-800">
              <h1 className="text-[28px] font-semibold tracking-normal text-neutral-950 dark:text-neutral-50">
                技能
              </h1>
              <div className="mt-3.5 flex min-w-0 items-center gap-4">
              <p className="min-w-0 flex-1 text-[14px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                管理内置与用户技能。启用后可在聊天中按需调用。
              </p>
              <div className="flex shrink-0 items-center gap-0.5">
                <IconButton
                  size="lg"
                  label="导入文件夹"
                  onClick={() => void handleImportSkill()}
                  data-tauri-drag-region="false"
                >
                  <FolderOpen size={17} />
                </IconButton>
                <IconButton
                  size="lg"
                  label="导入 zip"
                  onClick={() => void handleImportSkillZip()}
                  data-tauri-drag-region="false"
                >
                  <Download size={17} />
                </IconButton>
                <IconButton
                  size="lg"
                  label="打开 Skill 文件夹"
                  onClick={() => void handleOpenSkillFolder()}
                  data-tauri-drag-region="false"
                >
                  <ExternalLink size={17} />
                </IconButton>
                <IconButton
                  size="lg"
                  label="刷新列表"
                  onClick={() => void refreshChatSkills()}
                  disabled={skillsLoading}
                  data-tauri-drag-region="false"
                >
                  <RefreshCw size={17} className={skillsLoading ? 'animate-spin' : ''} />
                </IconButton>
              </div>
            </div>
          </div>

          {/* Tab 行 */}
          <div className="mt-5 flex items-center gap-1 border-b border-neutral-200 dark:border-neutral-800">
            {([['installed', '已安装'], ['store', '技能商店'], ['import', '本地导入'], ['advanced', '高级设置']] as const).map(([id, label]) => (
              <button
                key={id}
                type="button"
                onClick={() => setView(id)}
                data-tauri-drag-region="false"
                className={`relative px-3 py-2 text-[13px] font-medium transition-colors ${
                  view === id
                    ? 'text-neutral-900 dark:text-neutral-100'
                    : 'text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-200'
                }`}
              >
                {label}
                {view === id && (
                  <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-[#C56646] dark:bg-[#E39A78]" />
                )}
              </button>
            ))}
          </div>

          {view === 'store' ? (
            <div className="mt-5 flex min-h-[420px] flex-col">
              <SkillStoreBrowser onInstalled={() => void refreshChatSkills()} />
            </div>
          ) : view === 'import' ? (
            <div className="mt-5 space-y-4">
              <div className="flex flex-wrap gap-2">
                <Button onClick={() => void handleImportSkill()} data-tauri-drag-region="false">
                  <FolderOpen size={14} />
                  导入文件夹
                </Button>
                <Button onClick={() => void handleImportSkillZip()} data-tauri-drag-region="false">
                  <Download size={14} />
                  导入 zip
                </Button>
                <Button onClick={() => void handleOpenSkillFolder()} data-tauri-drag-region="false">
                  <ExternalLink size={14} />
                  打开 Skill 文件夹
                </Button>
              </div>
              <SkillUrlImport onInstalled={() => void refreshChatSkills()} />
              {skillError && (
                <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
                  {skillError}
                </div>
              )}
            </div>
          ) : view === 'advanced' ? (
          <section className="mt-5 overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
            <div className="flex w-full items-center gap-2 px-4 py-3">
              <Sliders size={15} className="shrink-0 text-neutral-400" />
              <span className="text-[13px] font-semibold text-neutral-800 dark:text-neutral-100">高级设置</span>
              <span className="text-[12px] text-neutral-400">自动匹配 · 降级模式 · 扫描路径</span>
            </div>
            <div>
              <div className="space-y-5 border-t border-neutral-200 px-4 py-4 dark:border-neutral-800">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium text-neutral-800 dark:text-neutral-100">自动匹配 Skill</div>
                    <p className="mt-0.5 text-[12px] text-neutral-500 dark:text-neutral-400">
                      允许模型根据 description 自动 activate skill
                    </p>
                  </div>
                  <Switch
                    checked={chatTools.skillAutoMatch !== false}
                    onChange={(skillAutoMatch) => persistChatTools({ skillAutoMatch })}
                    ariaLabel="自动匹配 Skill"
                  />
                </div>

                <div className="min-w-0">
                  <div className="min-w-0">
                    <div className="mb-1.5 text-[13px] font-medium text-neutral-800 dark:text-neutral-100">
                      无 Tools 降级模式
                    </div>
                    <Select
                      value={chatTools.skillFallbackMode || 'progressive'}
                      onChange={(value) => persistChatTools({ skillFallbackMode: value })}
                      options={[
                        { value: 'progressive', label: '渐进式（仅 catalog）' },
                        { value: 'skill_md_only', label: '仅 SKILL.md' },
                        { value: 'legacy_full_body', label: '旧版全量注入' },
                      ]}
                    />
                  </div>
                </div>

                <div className="min-w-0">
                  <div className="mb-1.5 text-[13px] font-medium text-neutral-800 dark:text-neutral-100">额外扫描路径</div>
                  <div className="space-y-1.5">
                    {chatTools.skillScanPaths.map((path, index) => (
                      <div key={`${path}-${index}`} className="flex items-center gap-1.5">
                        <input
                          type="text"
                          value={path}
                          onChange={(event) => {
                            const next = [...chatTools.skillScanPaths]
                            next[index] = event.target.value
                            persistChatTools({ skillScanPaths: next }, true)
                          }}
                          placeholder="/path/to/skills"
                          className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2.5 font-mono text-[12.5px] text-neutral-800 outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                          data-tauri-drag-region="false"
                        />
                        <IconButton
                          size="lg"
                          variant="danger"
                          label="移除路径"
                          onClick={() => {
                            const next = chatTools.skillScanPaths.filter((_, i) => i !== index)
                            persistChatTools({ skillScanPaths: next })
                            void refreshChatSkills(next)
                          }}
                          data-tauri-drag-region="false"
                        >
                          <Trash2 size={14} />
                        </IconButton>
                      </div>
                    ))}
                    <Button
                      onClick={async () => {
                        const selected = await open({ directory: true, multiple: false })
                        if (typeof selected === 'string') {
                          const next = [...chatTools.skillScanPaths, selected]
                          persistChatTools({ skillScanPaths: next })
                          void refreshChatSkills(next)
                        }
                      }}
                      data-tauri-drag-region="false"
                    >
                      <Plus size={13} />
                      添加扫描路径
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </section>
          ) : (
          <>
          {/* 搜索 */}
          <div className="relative mt-6">
            <Search size={16} className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-neutral-400" />
            <input
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索技能..."
              className="h-10 w-full rounded-md border border-neutral-200 bg-white pl-10 pr-4 text-[14px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              data-tauri-drag-region="false"
            />
          </div>

          {skillError && (
            <div className="mt-4 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
              {skillError}
            </div>
          )}

          {/* 技能列表 */}
          <div className="mt-6 space-y-5">
            {skillsLoading && skills.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center text-[13px] text-neutral-400">正在加载 Skill...</div>
            ) : skills.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center rounded-md border border-dashed border-neutral-200 px-6 text-center text-[13px] text-neutral-400 dark:border-neutral-800">
                暂无 Skill。可导入文件夹/zip，或打开 Skill 文件夹手动添加后刷新。
              </div>
            ) : (
              <>
                <SkillSection
                  title="工作区与个人技能"
                  emptyText={normalizedQuery ? '没有匹配的技能。' : '当前没有导入的技能。'}
                  skills={userSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  onPreview={handlePreviewSkill}
                />
                <SkillSection
                  title="内置技能"
                  note="随应用内置提供"
                  emptyText={normalizedQuery ? '没有匹配的内置技能。' : '当前没有内置技能。'}
                  skills={builtinSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  onPreview={handlePreviewSkill}
                  collapsible
                  defaultCollapsed
                />
                {/* 插件技能：无插件且未搜索时整段隐藏，避免空框霸占版面 */}
                {(pluginSkills.length > 0 || normalizedQuery) && (
                  <SkillSection
                    title="插件技能"
                    note="由扩展 → 插件 统一启用/关闭"
                    emptyText="没有匹配的插件技能。"
                    skills={pluginSkills}
                    disabledSkillIds={disabledSkillIds}
                    onToggleEnabled={handleToggleSkillEnabled}
                    onPreview={handlePreviewSkill}
                    manageLocked
                  />
                )}
              </>
            )}
          </div>
          </>
          )}
        </div>
      </main>

      {/* 预览弹窗 */}
      {selectedSkillPreview && (
        <div
          className="chat-motion-fade fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"
          data-tauri-drag-region="false"
          onClick={() => setSelectedSkillPreview(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="skill-preview-title"
            className="chat-motion-modal-in flex max-h-[80vh] w-full max-w-[640px] flex-col gap-3 overflow-hidden rounded-2xl border border-neutral-200 bg-white p-5 shadow-2xl dark:border-neutral-700 dark:bg-neutral-900"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-start gap-2">
              <Sparkles size={16} className="mt-0.5 shrink-0 text-[#C56646] dark:text-[#E39A78]" />
              <div className="min-w-0 flex-1">
                <h3 id="skill-preview-title" className="truncate text-[15px] font-semibold text-neutral-900 dark:text-neutral-100">
                  {selectedSkillPreview.name}
                </h3>
                <p className="mt-0.5 text-[12.5px] text-neutral-500 dark:text-neutral-400">
                  {selectedSkillPreview.description}
                </p>
              </div>
              <IconButton
                size="sm"
                label="关闭"
                onClick={() => setSelectedSkillPreview(null)}
                data-tauri-drag-region="false"
              >
                <X size={14} />
              </IconButton>
            </div>
            {selectedSkillPreview.recommendedTools.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {selectedSkillPreview.recommendedTools.map((tool) => (
                  <span
                    key={tool}
                    className="rounded-md bg-neutral-100 px-2 py-0.5 text-[11.5px] text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300"
                  >
                    {tool}
                  </span>
                ))}
              </div>
            )}
            <div className="custom-scrollbar max-h-[52vh] overflow-y-auto rounded-lg border border-neutral-200 bg-neutral-50 p-3 dark:border-neutral-800 dark:bg-neutral-950/50">
              <ChatMarkdown content={selectedSkillPreview.body} />
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
