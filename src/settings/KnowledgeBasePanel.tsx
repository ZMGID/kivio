// 知识库管理面板（Settings 页）：建库 / 选 embedding 模型 / 拖拽或选文件导入 /
// 文档列表 + 实时索引进度 / 删除 / 换 embedding 重建。
import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { Loader2, Trash2, Plus, RefreshCw, FileText, AlertCircle, CheckCircle2, Upload, FileSearch, Library, Pencil, Link2 } from 'lucide-react'
import { type ModelProvider, type DocumentProcessingConfig, type KnowledgeBaseConfig, isTauriRuntime } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select } from './components'
import { resolveModelInfo } from '../data/modelMatching'
import { KnowledgeRagPanel } from './KnowledgeRagPanel'
import {
  kbListLibraries,
  kbCreateLibrary,
  kbDeleteLibrary,
  kbRenameLibrary,
  kbListDocuments,
  kbDeleteDocument,
  kbUploadDocument,
  kbImportUrl,
  kbReindexLibrary,
  kbUpdateEmbedding,
  kbSetEmbedBatchSize,
  onKbIndex,
  type KnowledgeLibrary,
  type KnowledgeDocument,
} from '../chat/knowledgeBase'

const UPLOAD_EXTS = ['txt', 'text', 'log', 'csv', 'tsv', 'md', 'markdown', 'mdown', 'mkd', 'pdf', 'docx', 'xlsx', 'html', 'htm', 'png', 'jpg', 'jpeg', 'webp', 'bmp', 'tif', 'tiff', 'gif']

// embedding 请求批次：默认值 + 滑块范围，与后端 ingest::DEFAULT_EMBED_BATCH 及 1..=128 夹取一致。
const EMBED_BATCH_DEFAULT = 64
const EMBED_BATCH_MIN = 1
const EMBED_BATCH_MAX = 128

// Embedding 模型选择器：从 provider 的 availableModels 取建议（含未启用的 embedding
// 模型，如 bge-m3 / text-embedding-3-small），并允许自由输入——有些 provider 不在
// /models 里列出 embedding 模型。刻意不复用 ModelPairSelect（那个只列 enabledModels）。
function EmbeddingModelPicker({
  providers,
  providerId,
  model,
  onChange,
  lang,
  showBadges = true,
}: {
  providers: ModelProvider[]
  providerId: string
  model: string
  onChange: (providerId: string, model: string) => void
  lang: Lang
  showBadges?: boolean
}) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const enabled = providers.filter((p) => p.enabled !== false)
  const selected = enabled.find((p) => p.id === providerId)
  // Build the provider dropdown. If the bound providerId isn't among the
  // enabled providers (disabled, or deleted/never-saved → dangling binding),
  // surface it explicitly with a readable label so the Select never falls back
  // to showing the raw provider id.
  const options = [
    { value: '', label: t('选择提供商…', 'Pick provider…') },
    ...enabled.map((p) => ({ value: p.id, label: p.name || p.id })),
  ]
  if (providerId && !options.some((o) => o.value === providerId)) {
    const known = providers.find((p) => p.id === providerId)
    const label = known
      ? `${known.name || known.id}${known.enabled === false ? t('（已停用）', ' (disabled)') : ''}`
      : t('⚠ 供应商已删除，请重新选择', '⚠ provider missing — re-select')
    options.unshift({ value: providerId, label })
  }

  // Model dropdown: pick from THIS provider's configured (enabled) models —
  // the exact set the user curated in provider settings ("these models appear
  // in each feature's model selector"). Do NOT pull the full /models list.
  const configuredModels = selected?.enabledModels ?? []
  const modelOptions = [
    { value: '', label: t('选择 embedding 模型…', 'Pick embedding model…') },
    ...configuredModels.map((m) => ({ value: m, label: m })),
  ]
  // Keep an existing/legacy binding visible even if it's no longer enabled.
  if (model && !modelOptions.some((o) => o.value === model)) {
    modelOptions.push({ value: model, label: model })
  }

  // 解析模型信息（含嵌入维度/多语言/上下文），用于展示能力徽章。
  const info = model.trim() ? resolveModelInfo(model.trim(), selected?.modelOverrides) : null
  const isEmbedding = Boolean(info?.capabilities?.embedding || info?.dimensions)
  const ctxLabel = (n?: number) => (!n ? null : n >= 1000 ? `${Math.round(n / 1000)}K` : `${n}`)
  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <Select
          className="w-44"
          value={providerId}
          onChange={(pid) => onChange(pid, '')}
          options={options}
        />
        <Select
          className="w-64"
          value={model}
          onChange={(m) => onChange(providerId, m)}
          options={modelOptions}
        />
      </div>
      {showBadges && isEmbedding && (
        <div className="flex flex-wrap items-center gap-1">
          <span className="rounded-md border border-indigo-300 bg-indigo-50 px-1.5 py-0.5 text-[11px] font-medium text-indigo-600 dark:border-indigo-800 dark:bg-indigo-950/40 dark:text-indigo-300">
            {t('嵌入', 'Embedding')}
          </span>
          {info?.multilingual && <KbInfoPill>{t('多语言', 'Multilingual')}</KbInfoPill>}
          {info?.dimensions ? <KbInfoPill>{t(`${info.dimensions} 维`, `${info.dimensions}d`)}</KbInfoPill> : null}
          {ctxLabel(info?.contextWindow) ? <KbInfoPill>{ctxLabel(info?.contextWindow)}</KbInfoPill> : null}
          <KbInfoPill>RAG</KbInfoPill>
        </div>
      )}
    </div>
  )
}

function KbInfoPill({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-md bg-zinc-100 px-1.5 py-0.5 text-[11px] text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300">
      {children}
    </span>
  )
}

type Progress = { indexed: number; total: number }

export function KnowledgeBasePanel({
  providers,
  lang,
  docProcessing,
  onChangeDocProcessing,
  kbConfig,
  onChangeKbConfig,
  ragEnabled,
  onToggleRag,
}: {
  providers: ModelProvider[]
  lang: Lang
  docProcessing?: DocumentProcessingConfig
  onChangeDocProcessing: (next: DocumentProcessingConfig) => void
  kbConfig?: KnowledgeBaseConfig
  onChangeKbConfig: (next: KnowledgeBaseConfig) => void
  ragEnabled: boolean
  onToggleRag: (v: boolean) => void
}) {
  const t = useCallback((zh: string, en: string) => (lang === 'zh' ? zh : en), [lang])

  const [libraries, setLibraries] = useState<KnowledgeLibrary[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  // 右栏视图：某个库 / 新建 / RAG 设置（文档处理+分块+检索合并页）
  const [rightView, setRightView] = useState<'library' | 'new' | 'rag'>('library')
  const [docs, setDocs] = useState<KnowledgeDocument[]>([])
  const [progress, setProgress] = useState<Record<string, Progress>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 建库表单
  const [newName, setNewName] = useState('')
  const [newProviderId, setNewProviderId] = useState('')
  const [newModel, setNewModel] = useState('')

  // 选中库的 embedding 编辑草稿（改完点「应用并重建」才生效）
  const [editProviderId, setEditProviderId] = useState('')
  const [editModel, setEditModel] = useState('')

  // 每次 embedding 请求的片段数草稿；拖动即时反馈，松手/失焦持久化。
  // 未设置的库（存 0）显示为默认值 64，滑块永远是真实数字（不设"默认"特殊档）。
  const [batchDraft, setBatchDraft] = useState(EMBED_BATCH_DEFAULT)

  // 网址导入输入框
  const [urlInput, setUrlInput] = useState('')
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')

  // 拖拽导入：hover 高亮 + 全窗口接收（监听只在本面板挂载时存在）
  const [dragActive, setDragActive] = useState(false)

  const selected = libraries.find((l) => l.id === selectedId) ?? null

  const refreshLibraries = useCallback(async () => {
    try {
      const libs = await kbListLibraries()
      setLibraries(libs)
      setSelectedId((cur) => cur ?? libs[0]?.id ?? null)
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const refreshDocs = useCallback(async (kbId: string) => {
    try {
      setDocs(await kbListDocuments(kbId))
    } catch (e) {
      setError(String(e))
    }
  }, [])

  useEffect(() => {
    void refreshLibraries()
  }, [refreshLibraries])

  useEffect(() => {
    if (selectedId) void refreshDocs(selectedId)
    else setDocs([])
  }, [selectedId, refreshDocs])

  // 选中库变化时，把 embedding 草稿重置为该库当前配置。
  useEffect(() => {
    setEditProviderId(selected?.embeddingProviderId ?? '')
    setEditModel(selected?.embeddingModel ?? '')
    // 存 0（未设置）→ 显示默认 64；否则显示已存值。
    setBatchDraft(selected?.embedBatchSize ? selected.embedBatchSize : EMBED_BATCH_DEFAULT)
  }, [selected?.id, selected?.embeddingProviderId, selected?.embeddingModel, selected?.embedBatchSize])

  // 持久化片段数（松手/失焦时调）：写库 + 刷新，避免拖动过程每一格都落盘。
  const commitBatchSize = useCallback(
    async (kbId: string, size: number) => {
      try {
        await kbSetEmbedBatchSize(kbId, size)
        await refreshLibraries()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshLibraries],
  )

  // 实时索引进度：更新进度条；终态时刷新文档+库计数。
  const selectedRef = useRef<string | null>(null)
  selectedRef.current = selectedId
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    void onKbIndex((ev) => {
      setProgress((p) => ({ ...p, [ev.docId]: { indexed: ev.indexed, total: ev.total } }))
      if (ev.status !== 'indexing') {
        if (ev.kbId === selectedRef.current) void refreshDocs(ev.kbId)
        void refreshLibraries()
      }
    }).then((fn) => {
      // If we already unmounted before the listener resolved, detach immediately
      // — otherwise the listener leaks (cleanup ran while unlisten was undefined).
      if (cancelled) fn()
      else unlisten = fn
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [refreshDocs, refreshLibraries])

  const handleCreate = async () => {
    if (!newName.trim() || !newProviderId || !newModel) {
      setError(t('请填写名称并选择 embedding 模型', 'Enter a name and pick an embedding model'))
      return
    }
    setBusy(true)
    setError(null)
    try {
      const lib = await kbCreateLibrary(newName.trim(), newProviderId, newModel)
      setNewName('')
      await refreshLibraries()
      setSelectedId(lib.id)
      setRightView('library')
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  // 导入一批文件路径：每个独立 try,一个失败不拖累其余,最后统一报告。
  // 选文件按钮与拖拽共用此逻辑。
  const uploadPaths = useCallback(
    async (kbId: string, paths: string[]) => {
      if (paths.length === 0) return
      setBusy(true)
      setError(null)
      const failures: string[] = []
      for (const path of paths) {
        try {
          await kbUploadDocument(kbId, path)
        } catch (e) {
          const name = path.split(/[\\/]/).pop() || path
          failures.push(`${name}: ${e}`)
        }
      }
      await refreshDocs(kbId).catch(() => {})
      await refreshLibraries()
      setBusy(false)
      if (failures.length > 0) {
        setError(t(`${failures.length} 个文件导入失败：`, `${failures.length} file(s) failed: `) + failures.join('; '))
      }
    },
    [refreshDocs, refreshLibraries, t],
  )

  const handleUpload = async () => {
    if (!selectedId) return
    let picked: string | string[] | null
    try {
      picked = await open({
        multiple: true,
        filters: [{ name: 'Documents', extensions: UPLOAD_EXTS }],
      })
    } catch (e) {
      setError(String(e))
      return
    }
    if (!picked) return
    const paths = Array.isArray(picked) ? picked : [picked]
    await uploadPaths(selectedId, paths)
  }

  // 拖拽导入：Tauri 的 drag-drop 事件才带真实文件系统路径（HTML5 File 拿不到），
  // 事件是窗口级的。此监听只在 KB 详情面板挂载时存在，所以拖文件进来即视为导入——
  // 不做坐标命中测试：event.position 在 macOS 是逻辑坐标、Windows 是物理坐标，
  // 二者与 getBoundingClientRect 的换算基准不一致（旧的除以 DPR 命中测试在 macOS
  // Retina 上必然落空，导致拖拽在 macOS 完全失效）。与 InputBar 附件拖拽一致，全窗口接收。
  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined

    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (cancelled) return
        const p = event.payload
        if (p.type === 'enter' || p.type === 'over') {
          setDragActive(Boolean(selectedRef.current))
          return
        }
        if (p.type === 'leave') {
          setDragActive(false)
          return
        }
        if (p.type === 'drop') {
          setDragActive(false)
          const kbId = selectedRef.current
          if (!kbId) return
          const paths = p.paths.filter((path) => {
            const ext = path.split('.').pop()?.toLowerCase() ?? ''
            return UPLOAD_EXTS.includes(ext)
          })
          if (paths.length === 0) {
            setError(t('拖入的文件类型不受支持', 'Dropped file type is not supported'))
            return
          }
          void uploadPaths(kbId, paths)
        }
      })
      .then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      .catch((err) => console.error('KB drag-drop listen failed:', err))

    return () => {
      cancelled = true
      setDragActive(false)
      unlisten?.()
    }
  }, [uploadPaths, t])

  const handleImportUrl = async () => {
    if (!selectedId) return
    const url = urlInput.trim()
    if (!url) return
    setBusy(true)
    setError(null)
    try {
      await kbImportUrl(selectedId, url)
      setUrlInput('')
      await refreshDocs(selectedId).catch(() => {})
      await refreshLibraries()
    } catch (e) {
      setError(t('网址导入失败：', 'URL import failed: ') + String(e))
    } finally {
      setBusy(false)
    }
  }

  const handleDeleteLibrary = async (kbId: string) => {
    if (!confirm(t('删除该知识库及其所有文档？', 'Delete this knowledge base and all its documents?'))) return
    try {
      await kbDeleteLibrary(kbId)
      if (selectedId === kbId) setSelectedId(null)
      await refreshLibraries()
    } catch (e) {
      setError(String(e))
    }
  }

  const handleDeleteDoc = async (docId: string) => {
    if (!selectedId) return
    try {
      await kbDeleteDocument(selectedId, docId)
      await refreshDocs(selectedId)
      await refreshLibraries()
    } catch (e) {
      setError(String(e))
    }
  }

  const handleRename = async (kbId: string, name: string) => {
    const trimmed = name.trim()
    const current = libraries.find((l) => l.id === kbId)?.name
    if (!trimmed || trimmed === current) return
    try {
      await kbRenameLibrary(kbId, trimmed)
      setRenamingId(null)
      await refreshLibraries()
    } catch (e) {
      setError(String(e))
    }
  }

  const startRenaming = (kbId: string, currentName: string) => {
    setRenamingId(kbId)
    setRenameDraft(currentName)
  }

  const cancelRenaming = () => {
    setRenamingId(null)
    setRenameDraft('')
  }

  const handleChangeEmbedding = async (providerId: string, model: string) => {
    if (!selected || !providerId || !model) return
    if (providerId === selected.embeddingProviderId && model === selected.embeddingModel) return
    if (
      !confirm(
        t(
          '更换 embedding 模型会重建整个知识库索引（重新调用 embedding，可能耗时与产生费用）。继续？',
          'Changing the embedding model rebuilds the whole index (re-embeds every chunk — may take time and cost). Continue?'
        )
      )
    )
      return
    setBusy(true)
    try {
      await kbUpdateEmbedding(selected.id, providerId, model)
      await refreshLibraries()
      await refreshDocs(selected.id)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const handleReindex = async () => {
    if (!selected) return
    if (!confirm(t('重建该知识库的全部索引？', 'Rebuild the whole index for this library?'))) return
    setBusy(true)
    try {
      await kbReindexLibrary(selected.id)
      await refreshDocs(selected.id)
      await refreshLibraries()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="kb-panel-root flex min-h-full items-stretch gap-0">
      {/* 左侧二级侧边栏：分隔线拉满可用高度 */}
      <nav className="relative flex h-full min-h-full w-44 shrink-0 flex-col self-stretch pr-3">
        <div
          className="pointer-events-none absolute inset-y-0 right-0 w-px bg-zinc-200/80 dark:bg-zinc-800"
          aria-hidden
        />
        <div className="space-y-0.5">
        <button
          type="button"
          onClick={() => setRightView('rag')}
          className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition ${
            rightView === 'rag'
              ? 'bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300'
              : 'text-zinc-700 hover:bg-zinc-100 dark:text-zinc-200 dark:hover:bg-zinc-800'
          }`}
        >
          <FileSearch size={14} className="shrink-0 text-zinc-400" /> {t('知识库（RAG）', 'RAG settings')}
        </button>

        <div className="my-1.5 border-t border-zinc-200/70 dark:border-zinc-800" />

        <div className="px-2 pb-1 pt-1 text-[11px] font-medium uppercase tracking-wide text-zinc-400">
          {t('知识库', 'Libraries')}
        </div>
        {libraries.map((lib) => {
          const active = rightView === 'library' && lib.id === selectedId
          return (
            <button
              key={lib.id}
              type="button"
              onClick={() => {
                setSelectedId(lib.id)
                setRightView('library')
              }}
              className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition ${
                active
                  ? 'bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300'
                  : 'text-zinc-700 hover:bg-zinc-100 dark:text-zinc-200 dark:hover:bg-zinc-800'
              }`}
            >
              <Library size={14} className="shrink-0 text-zinc-400" />
              <span className="min-w-0 flex-1 truncate">{lib.name}</span>
              <span className="shrink-0 text-xs text-zinc-400" title={t('文档数', 'Documents')}>
                {lib.docCount}
              </span>
            </button>
          )
        })}
        <button
          type="button"
          onClick={() => setRightView('new')}
          className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition ${
            rightView === 'new'
              ? 'bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300'
              : 'text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800'
          }`}
        >
          <Plus size={14} className="shrink-0" /> {t('新建知识库', 'New library')}
        </button>
        </div>
      </nav>

      {/* 右侧内容 */}
      <div className="min-w-0 flex-1 space-y-4 pl-4">
        {error && (
          <div className="flex items-center gap-2 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600 dark:bg-red-950/40 dark:text-red-400">
            <AlertCircle size={14} />
            <span className="flex-1">{error}</span>
            <button type="button" onClick={() => setError(null)} className="text-xs underline">
              {t('关闭', 'dismiss')}
            </button>
          </div>
        )}

        {/* 新建知识库 */}
        {rightView === 'new' && (
          <SettingsGroup title={t('新建知识库', 'New knowledge base')}>
            <div className="flex flex-wrap items-center gap-2 py-2">
              <Input
                value={newName}
                onChange={setNewName}
                placeholder={t('知识库名称', 'Library name')}
                className="w-44"
              />
              <EmbeddingModelPicker
                providers={providers}
                providerId={newProviderId}
                model={newModel}
                onChange={(p, m) => {
                  setNewProviderId(p)
                  setNewModel(m)
                }}
                lang={lang}
              />
              <button
                type="button"
                disabled={busy}
                onClick={handleCreate}
                className="inline-flex items-center gap-1 rounded-lg bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
              >
                <Plus size={14} /> {t('创建', 'Create')}
              </button>
            </div>
            <p className="px-1 pb-1 text-xs text-zinc-500">
              {t(
                'embedding 模型决定向量维度，建库后更换需重建索引。需选用支持 /embeddings 接口的模型。',
                'The embedding model fixes the vector dimension; changing it later rebuilds the index. Use a model that serves the /embeddings endpoint.'
              )}
            </p>
          </SettingsGroup>
        )}

        {/* RAG 设置（文档处理 + 分块 + 检索） */}
        {rightView === 'rag' && (
          <KnowledgeRagPanel
            providers={providers}
            lang={lang}
            docProcessing={docProcessing}
            onChangeDocProcessing={onChangeDocProcessing}
            kbConfig={kbConfig}
            onChangeKbConfig={onChangeKbConfig}
            ragEnabled={ragEnabled}
            onToggleRag={onToggleRag}
          />
        )}

        {/* 选中库为空时的提示 */}
        {rightView === 'library' && !selected && (
          <p className="px-1 pt-6 text-sm text-zinc-500">
            {t(
              '从左侧选择一个知识库，或点「新建知识库」。',
              'Pick a library on the left, or click “New library”.'
            )}
          </p>
        )}

        {/* 选中库详情 */}
        {rightView === 'library' && selected && (
          <div className="space-y-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex-1">
                {renamingId === selected.id ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <Input
                      value={renameDraft}
                      onChange={setRenameDraft}
                      className="w-48"
                      placeholder={t('知识库名称', 'Library name')}
                    />
                    <button
                      type="button"
                      disabled={busy || !renameDraft.trim()}
                      onClick={() => void handleRename(selected.id, renameDraft)}
                      className="rounded-lg bg-indigo-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
                    >
                      {t('保存', 'Save')}
                    </button>
                    <button
                      type="button"
                      onClick={cancelRenaming}
                      className="rounded-lg px-2.5 py-1.5 text-xs text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                    >
                      {t('取消', 'Cancel')}
                    </button>
                  </div>
                ) : (
                  <h2 className="truncate text-base font-semibold text-zinc-900 dark:text-zinc-50">
                    {selected.name}
                  </h2>
                )}
                <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
                  {t(
                    `${selected.docCount} 个文档 · ${selected.chunkCount} 块`,
                    `${selected.docCount} docs · ${selected.chunkCount} chunks`,
                  )}
                  {selected.embeddingDim > 0
                    ? t(` · ${selected.embeddingDim} 维`, ` · ${selected.embeddingDim}d`)
                    : ''}
                </p>
              </div>
              {renamingId !== selected.id && (
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    onClick={() => startRenaming(selected.id, selected.name)}
                    className="rounded-lg p-1.5 text-zinc-400 transition hover:bg-zinc-100 hover:text-zinc-600 dark:hover:bg-zinc-800 dark:hover:text-zinc-200"
                    title={t('重命名', 'Rename')}
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    type="button"
                    onClick={() => handleDeleteLibrary(selected.id)}
                    className="rounded-lg p-1.5 text-zinc-400 transition hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-950/40 dark:hover:text-red-400"
                    title={t('删除库', 'Delete library')}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              )}
            </div>

            <SettingsGroup title={t('Embedding 模型', 'Embedding model')}>
              <div className="space-y-2 py-2">
                <EmbeddingModelPicker
                  providers={providers}
                  providerId={editProviderId}
                  model={editModel}
                  onChange={(p, m) => {
                    setEditProviderId(p)
                    setEditModel(m)
                  }}
                  lang={lang}
                  showBadges
                />
                {(editProviderId !== selected.embeddingProviderId || editModel !== selected.embeddingModel) && (
                  <button
                    type="button"
                    disabled={busy || !editProviderId || !editModel}
                    onClick={() => handleChangeEmbedding(editProviderId, editModel)}
                    className="inline-flex items-center gap-1 rounded-lg bg-amber-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-amber-700 disabled:opacity-50"
                  >
                    {t('应用并重建索引', 'Apply & rebuild index')}
                  </button>
                )}

                <div className="border-t border-zinc-100 pt-3 dark:border-zinc-800">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-sm text-zinc-700 dark:text-zinc-200">
                      {t('请求文档片段数量', 'Fragments per request')}
                    </span>
                    <span className="rounded-md border border-zinc-200 bg-white px-2 py-0.5 font-mono text-xs text-zinc-700 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-200">
                      {batchDraft}
                    </span>
                  </div>
                  <input
                    type="range"
                    min={EMBED_BATCH_MIN}
                    max={EMBED_BATCH_MAX}
                    step={1}
                    value={batchDraft}
                    onChange={(e) => setBatchDraft(Number(e.target.value))}
                    onPointerUp={() => void commitBatchSize(selected.id, batchDraft)}
                    onBlur={() => void commitBatchSize(selected.id, batchDraft)}
                    className="mt-2 w-full"
                    style={{ accentColor: 'var(--accent)' }}
                  />
                  <div className="flex justify-between text-[11px] text-zinc-400">
                    <span>{EMBED_BATCH_MIN}</span>
                    <span>{EMBED_BATCH_MAX}</span>
                  </div>
                  <p className="mt-1.5 text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">
                    {t(
                      `每次向量化请求打包多少个文档片段（默认 ${EMBED_BATCH_DEFAULT}）。若 embedding 服务报“批量过大/条数超限”可调小，只影响后续索引、无需重建。`,
                      `How many chunks each embedding request packs (default ${EMBED_BATCH_DEFAULT}). Lower it if the embedding service rejects large batches. Affects future indexing only — no rebuild.`,
                    )}
                  </p>
                </div>
              </div>
            </SettingsGroup>

            <SettingsGroup title={t('文档', 'Documents')}>
              <div className="space-y-3 py-2">
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    disabled={busy}
                    onClick={handleUpload}
                    className="inline-flex items-center gap-1 rounded-lg bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
                  >
                    <Upload size={14} /> {t('导入文档', 'Add documents')}
                  </button>
                  <button
                    type="button"
                    disabled={busy || docs.length === 0}
                    onClick={handleReindex}
                    className="inline-flex items-center gap-1 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm hover:bg-zinc-50 disabled:opacity-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
                  >
                    <RefreshCw size={14} /> {t('重建索引', 'Rebuild')}
                  </button>
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  <div className="relative min-w-[220px] flex-1">
                    <Link2 size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
                    <Input
                      className="!pl-10"
                      value={urlInput}
                      onChange={setUrlInput}
                      placeholder={t('粘贴网址导入（https://…）', 'Paste URL (https://…)')}
                      mono
                    />
                  </div>
                  <button
                    type="button"
                    disabled={busy || !urlInput.trim()}
                    onClick={handleImportUrl}
                    className="inline-flex items-center gap-1 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm hover:bg-zinc-50 disabled:opacity-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
                  >
                    <Plus size={14} /> {t('导入网址', 'Add URL')}
                  </button>
                </div>

                {/* 大虚线框常驻：点击选文件 / 拖拽到此；拖拽悬停时高亮框本身。 */}
                <button
                  type="button"
                  disabled={busy}
                  onClick={handleUpload}
                  className={`flex w-full flex-col items-center gap-2 rounded-lg border border-dashed px-4 py-8 text-center transition disabled:opacity-50 ${
                    dragActive
                      ? 'border-indigo-400 bg-indigo-50/70 dark:border-indigo-500 dark:bg-indigo-950/40'
                      : 'border-zinc-300 bg-zinc-50/50 hover:border-indigo-300 hover:bg-indigo-50/40 dark:border-zinc-700 dark:bg-zinc-900/30 dark:hover:border-indigo-800 dark:hover:bg-indigo-950/20'
                  }`}
                >
                  <Upload size={20} className={dragActive ? 'text-indigo-500' : 'text-zinc-400'} />
                  <span className={`text-sm font-medium ${dragActive ? 'text-indigo-600 dark:text-indigo-300' : 'text-zinc-600 dark:text-zinc-300'}`}>
                    {dragActive ? t('松开以导入', 'Drop to import') : t('点击导入文档', 'Click to add documents')}
                  </span>
                  <span className="max-w-md text-xs leading-relaxed text-zinc-400">
                    {t(
                      '点击或拖拽文件到此处；支持 txt / md / pdf / docx / xlsx / html、图片（需开启 OCR），或使用上方网址导入',
                      'Click or drag files here; txt, md, pdf, docx, xlsx, html, images (OCR required), or import a URL above',
                    )}
                  </span>
                </button>

                {docs.length > 0 && (
                  <div className="divide-y divide-zinc-100 overflow-hidden rounded-lg border border-zinc-200 dark:divide-zinc-800 dark:border-zinc-700">
                    {docs.map((doc) => (
                      <DocRow
                        key={doc.id}
                        doc={doc}
                        progress={progress[doc.id]}
                        lang={lang}
                        onDelete={() => handleDeleteDoc(doc.id)}
                      />
                    ))}
                  </div>
                )}
              </div>
            </SettingsGroup>
          </div>
        )}
      </div>
    </div>
  )
}

function DocRow({
  doc,
  progress,
  lang,
  onDelete,
}: {
  doc: KnowledgeDocument
  progress?: Progress
  lang: Lang
  onDelete: () => void
}) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const indexing = doc.status === 'indexing'
  // 有 total 才是「向量化」阶段（可确定进度）；total 未知＝还在解析/OCR（不确定进度）。
  const determinate = indexing && !!progress && progress.total > 0
  const pct = determinate ? Math.round((progress!.indexed / progress!.total) * 100) : 0
  return (
    <div className="px-3 py-2 text-sm">
      <div className="flex items-center gap-2">
        <FileText size={14} className="shrink-0 text-zinc-400" />
        <span className="flex-1 truncate" title={doc.name}>
          {doc.name}
        </span>
        {indexing && (
          <span className="flex items-center gap-1 text-xs text-indigo-500">
            <Loader2 size={12} className="animate-spin" />
            {determinate ? `${progress!.indexed}/${progress!.total}` : t('处理中', 'processing')}
          </span>
        )}
        {doc.status === 'ready' && (
          <span className="flex items-center gap-1 text-xs text-emerald-500">
            <CheckCircle2 size={12} /> {t(`${doc.chunkCount} 块`, `${doc.chunkCount}`)}
          </span>
        )}
        {doc.status === 'error' && (
          <span className="flex items-center gap-1 text-xs text-red-500" title={doc.error ?? ''}>
            <AlertCircle size={12} /> {t('失败', 'error')}
          </span>
        )}
        <button
          type="button"
          onClick={onDelete}
          className="shrink-0 rounded p-1 text-zinc-400 hover:bg-zinc-100 hover:text-red-500 dark:hover:bg-zinc-800"
          title={t('删除文档', 'Delete document')}
        >
          <Trash2 size={13} />
        </button>
      </div>
      {indexing && (
        <div className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800">
          {determinate ? (
            <div
              className="h-full rounded-full bg-indigo-500 transition-[width] duration-[var(--kv-dur-normal)] ease-[var(--kv-ease-out)]"
              style={{ width: `${pct}%` }}
            />
          ) : (
            // 解析/OCR 阶段无逐步进度：用脉动条表示「进行中但无确定百分比」。
            <div className="h-full w-1/3 animate-pulse rounded-full bg-indigo-400/70" />
          )}
        </div>
      )}
    </div>
  )
}
