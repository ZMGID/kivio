import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
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
import { usesNativeTitlebar } from './platform'
import { Select } from '../settings/components'

interface SkillCenterProps {
  /** 返回對話檢視 */
  onClose: () => void
  /** Skill 啟用狀態 / 列表變化後通知 Chat 重新整理其技能列表 */
  onSkillsChanged?: () => void
}

function defaultChatTools(): ChatToolsConfig {
  return {
    enabled: false,
    servers: [],
    skillScanPaths: [],
    skillAutoMatch: true,
    skillFallbackMode: 'progressive',
    skillScriptAllowlist: ['python3', 'bash', 'sh', 'node'],
    disabledSkillIds: [],
    maxToolRounds: 20,
    toolTimeoutMs: 60_000,
    mcpIdleTimeoutMs: 600_000,
    maxToolOutputChars: null,
    approvalPolicy: 'readonly_auto_sensitive_confirm',
    nativeTools: defaultNativeTools(),
  }
}

function isBuiltinSkill(skill: SkillMeta): boolean {
  return skill.source === 'builtin'
}

function skillSourceLabel(skill: SkillMeta): string {
  if (skill.source === 'builtin') return '內建'
  if (skill.source === 'external') return '工作區'
  return '個人'
}

function skillMatches(skill: SkillMeta, query: string): boolean {
  if (!query) return true
  return (
    skill.name.toLowerCase().includes(query) ||
    (skill.description ?? '').toLowerCase().includes(query)
  )
}

/** 自帶樣式的開關：明暗對比清晰，不依賴設定面板的 CSS 變數作用域 */
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
}: {
  skill: SkillMeta
  enabled: boolean
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
}) {
  return (
    <div
      className={`flex min-w-0 items-center gap-4 px-5 py-4 transition-colors hover:bg-neutral-50 dark:hover:bg-neutral-900/50 ${
        enabled ? '' : 'opacity-60'
      }`}
    >
      <button
        type="button"
        onClick={() => onPreview(skill.id)}
        className="flex min-w-0 flex-1 items-center gap-4 text-left"
        data-tauri-drag-region="false"
        title="檢視完整內容"
      >
        <span className="grid size-11 shrink-0 place-items-center rounded-full border border-neutral-200 bg-white text-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-500">
          <Box size={18} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[15px] font-semibold text-neutral-900 dark:text-neutral-100">
            {skill.name}
          </span>
          <span className="mt-0.5 block truncate text-[13px] text-neutral-500 dark:text-neutral-400">
            {skill.description || '未設定描述'}
          </span>
        </span>
      </button>
      <span className="shrink-0 text-[13px] text-neutral-400 dark:text-neutral-500">{skillSourceLabel(skill)}</span>
      <Switch checked={enabled} onChange={(next) => onToggleEnabled(skill.id, next)} ariaLabel={`啟用 ${skill.name}`} />
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
}: {
  title: string
  note?: string
  emptyText: string
  skills: SkillMeta[]
  disabledSkillIds: string[]
  onToggleEnabled: (skillId: string, enabled: boolean) => void
  onPreview: (skillId: string) => void
}) {
  return (
    <section className="space-y-2.5">
      <div className="flex min-w-0 items-center gap-3 px-1">
        <h3 className="text-[15px] font-semibold text-neutral-700 dark:text-neutral-200">{title}</h3>
        <span className="text-[14px] font-medium text-neutral-400">{skills.length}</span>
        {note && <span className="ml-auto truncate text-[12.5px] text-neutral-400">{note}</span>}
      </div>
      {skills.length === 0 ? (
        <div className="grid min-h-[88px] place-items-center rounded-2xl border border-dashed border-neutral-200 text-[13px] text-neutral-400 dark:border-neutral-800">
          {emptyText}
        </div>
      ) : (
        <div className="overflow-hidden rounded-2xl border border-neutral-200 bg-white shadow-sm dark:border-neutral-800 dark:bg-neutral-950/40 [&>*+*]:border-t [&>*+*]:border-neutral-100 dark:[&>*+*]:border-neutral-800/70">
          {skills.map((skill) => (
            <SkillRow
              key={skill.id}
              skill={skill}
              enabled={!disabledSkillIds.includes(skill.id)}
              onToggleEnabled={onToggleEnabled}
              onPreview={onPreview}
            />
          ))}
        </div>
      )}
    </section>
  )
}

export function SkillCenter({ onClose, onSkillsChanged }: SkillCenterProps) {
  const [settings, setSettings] = useState<Settings | null>(null)
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [skillsLoading, setSkillsLoading] = useState(false)
  const [skillError, setSkillError] = useState('')
  const [query, setQuery] = useState('')
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const [selectedSkillPreview, setSelectedSkillPreview] = useState<SkillDetail | null>(null)

  // 進階設定摺疊時內容仍在 DOM（用於 chat-motion-reveal 高度動畫），用 inert 讓其退出 tab 序 / a11y 樹，
  // 避免鍵盤 Tab 進入視覺摺疊的表單控制元件（WCAG 2.1.1）。
  const advancedRef = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    const el = advancedRef.current
    if (el) el.inert = !advancedOpen
  }, [advancedOpen])

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
        setSkillError(result.error || 'Skill 列表載入失敗')
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
        const loaded = await api.getSettings()
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
      const saved = await api.saveSettings(next)
      settingsRef.current = saved
      onSkillsChanged?.()
    } catch (err) {
      setSkillError(err instanceof Error ? err.message : String(err))
    }
  }, [onSkillsChanged])

  // 更新 chatTools：本機立即生效，再持久化（文字類編輯防抖，開關/下拉立即儲存）
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
        setSkillError(result.error || '讀取 Skill 失敗')
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
        setSkillError(result.error || '匯入 Skill 失敗')
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
        setSkillError(result.error || '匯入 Skill 失敗')
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
        setSkillError(result.error || '開啟 Skill 資料夾失敗')
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
  const userSkills = useMemo(
    () => skills.filter((skill) => !isBuiltinSkill(skill) && skillMatches(skill, normalizedQuery)),
    [skills, normalizedQuery],
  )

  const headerActionClass =
    'grid size-9 shrink-0 place-items-center rounded-lg text-neutral-500 hover:bg-neutral-100 hover:text-neutral-800 disabled:opacity-50 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100'

  return (
    <div className="assistant-center-root flex h-full min-h-0 flex-col text-neutral-900 dark:text-neutral-100">
      {/* 頂欄：與聊天主區同底色、無分隔；可拖拽，右側避開視窗按鈕 */}
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
      <main className="custom-scrollbar min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-[1040px] px-9 pb-10 pt-7">
            {/* 頭部：標題 + 副標題 + 圖示動作 */}
            <div className="border-b border-neutral-200/80 pb-6 dark:border-neutral-800/80">
              <h1 className="text-[32px] font-bold leading-none tracking-tight text-neutral-950 dark:text-neutral-50">
                技能
              </h1>
              <div className="mt-3.5 flex min-w-0 items-center gap-4">
              <p className="min-w-0 flex-1 text-[14px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                管理內建與使用者技能。啟用後可在聊天中按需呼叫。
              </p>
              <div className="flex shrink-0 items-center gap-0.5">
                <button
                  type="button"
                  onClick={() => void handleImportSkill()}
                  className={headerActionClass}
                  title="匯入資料夾"
                  aria-label="匯入資料夾"
                  data-tauri-drag-region="false"
                >
                  <FolderOpen size={17} />
                </button>
                <button
                  type="button"
                  onClick={() => void handleImportSkillZip()}
                  className={headerActionClass}
                  title="匯入 zip"
                  aria-label="匯入 zip"
                  data-tauri-drag-region="false"
                >
                  <Download size={17} />
                </button>
                <button
                  type="button"
                  onClick={() => void handleOpenSkillFolder()}
                  className={headerActionClass}
                  title="開啟 Skill 資料夾"
                  aria-label="開啟 Skill 資料夾"
                  data-tauri-drag-region="false"
                >
                  <ExternalLink size={17} />
                </button>
                <button
                  type="button"
                  onClick={() => void refreshChatSkills()}
                  disabled={skillsLoading}
                  className={headerActionClass}
                  title="重新整理列表"
                  aria-label="重新整理列表"
                  data-tauri-drag-region="false"
                >
                  <RefreshCw size={17} className={skillsLoading ? 'animate-spin' : ''} />
                </button>
              </div>
            </div>
          </div>

          {/* 搜尋 */}
          <div className="relative mt-6">
            <Search size={17} className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-neutral-400" />
            <input
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜尋技能..."
              className="h-11 w-full rounded-xl border border-neutral-200 bg-white pl-11 pr-4 text-[14px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              data-tauri-drag-region="false"
            />
          </div>

          {skillError && (
            <div className="mt-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
              {skillError}
            </div>
          )}

          {/* 進階設定（預設摺疊） */}
          <section className="mt-4 overflow-hidden rounded-xl border border-neutral-200 dark:border-neutral-800">
            <button
              type="button"
              onClick={() => setAdvancedOpen((open) => !open)}
              className="flex w-full items-center gap-2 px-4 py-3 text-left hover:bg-neutral-50 dark:hover:bg-neutral-900/60"
              aria-expanded={advancedOpen}
              data-tauri-drag-region="false"
            >
              <Sliders size={15} className="shrink-0 text-neutral-400" />
              <span className="text-[13px] font-semibold text-neutral-800 dark:text-neutral-100">進階設定</span>
              <span className="text-[12px] text-neutral-400">自動匹配 · 降級模式 · 直譯器白名單 · 掃描路徑</span>
              <ChevronDown
                size={16}
                className={`ml-auto shrink-0 text-neutral-400 transition-transform duration-[var(--kv-dur-fast)] ease-[var(--kv-ease-standard)] ${advancedOpen ? 'rotate-180' : ''}`}
              />
            </button>
            <div ref={advancedRef} className={`chat-motion-reveal ${advancedOpen ? 'is-open' : ''}`}>
              <div className="space-y-5 border-t border-neutral-200 px-4 py-4 dark:border-neutral-800">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium text-neutral-800 dark:text-neutral-100">自動匹配 Skill</div>
                    <p className="mt-0.5 text-[12px] text-neutral-500 dark:text-neutral-400">
                      允許模型根據 description 自動 activate skill
                    </p>
                  </div>
                  <Switch
                    checked={chatTools.skillAutoMatch !== false}
                    onChange={(skillAutoMatch) => persistChatTools({ skillAutoMatch })}
                    ariaLabel="自動匹配 Skill"
                  />
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <div className="min-w-0">
                    <div className="mb-1.5 text-[13px] font-medium text-neutral-800 dark:text-neutral-100">
                      無 Tools 降級模式
                    </div>
                    <Select
                      value={chatTools.skillFallbackMode || 'progressive'}
                      onChange={(value) => persistChatTools({ skillFallbackMode: value })}
                      options={[
                        { value: 'progressive', label: '漸進式（僅 catalog）' },
                        { value: 'skill_md_only', label: '僅 SKILL.md' },
                        { value: 'legacy_full_body', label: '舊版全量注入' },
                      ]}
                    />
                  </div>
                  <div className="min-w-0">
                    <div className="mb-1.5 text-[13px] font-medium text-neutral-800 dark:text-neutral-100">
                      指令碼直譯器白名單
                    </div>
                    <input
                      type="text"
                      value={(chatTools.skillScriptAllowlist || []).join(', ')}
                      onChange={(event) =>
                        persistChatTools(
                          {
                            skillScriptAllowlist: event.target.value
                              .split(',')
                              .map((item) => item.trim())
                              .filter(Boolean),
                          },
                          true,
                        )
                      }
                      placeholder="python3, bash, sh, node"
                      className="h-9 w-full rounded-md border border-neutral-200 bg-white px-2.5 font-mono text-[12.5px] text-neutral-800 outline-none focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                      data-tauri-drag-region="false"
                    />
                  </div>
                </div>

                <div className="min-w-0">
                  <div className="mb-1.5 text-[13px] font-medium text-neutral-800 dark:text-neutral-100">額外掃描路徑</div>
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
                        <button
                          type="button"
                          className="grid size-9 shrink-0 place-items-center rounded-md text-neutral-400 hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-950/30 dark:hover:text-red-300"
                          onClick={() => {
                            const next = chatTools.skillScanPaths.filter((_, i) => i !== index)
                            persistChatTools({ skillScanPaths: next })
                            void refreshChatSkills(next)
                          }}
                          data-tauri-drag-region="false"
                          aria-label="移除路徑"
                        >
                          <Trash2 size={14} />
                        </button>
                      </div>
                    ))}
                    <button
                      type="button"
                      className="inline-flex items-center gap-1.5 rounded-md border border-neutral-200 bg-white px-2.5 py-1.5 text-[12.5px] font-medium text-neutral-600 hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800"
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
                      新增掃描路徑
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </section>

          {/* 技能列表 */}
          <div className="mt-7 space-y-7">
            {skillsLoading && skills.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center text-[13px] text-neutral-400">正在載入 Skill...</div>
            ) : skills.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center rounded-2xl border border-dashed border-neutral-200 px-6 text-center text-[13px] text-neutral-400 dark:border-neutral-800">
                暫無 Skill。可匯入資料夾/zip，或開啟 Skill 資料夾手動新增後重新整理。
              </div>
            ) : (
              <>
                <SkillSection
                  title="內建技能"
                  note="隨應用內建提供"
                  emptyText={normalizedQuery ? '沒有匹配的內建技能。' : '當前沒有內建技能。'}
                  skills={builtinSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  onPreview={handlePreviewSkill}
                />
                <SkillSection
                  title="工作區與個人技能"
                  emptyText={normalizedQuery ? '沒有匹配的技能。' : '當前沒有匯入的技能。'}
                  skills={userSkills}
                  disabledSkillIds={disabledSkillIds}
                  onToggleEnabled={handleToggleSkillEnabled}
                  onPreview={handlePreviewSkill}
                />
              </>
            )}
          </div>
        </div>
      </main>

      {/* 預覽彈窗 */}
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
              <button
                type="button"
                className="grid size-7 shrink-0 place-items-center rounded-md text-neutral-400 hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
                onClick={() => setSelectedSkillPreview(null)}
                data-tauri-drag-region="false"
                aria-label="關閉"
              >
                <X size={14} />
              </button>
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
