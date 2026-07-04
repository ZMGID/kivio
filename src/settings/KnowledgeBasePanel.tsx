// 知識庫管理面板（Settings 頁）：建庫 / 選 embedding 模型 / 拖拽或選檔案匯入 /
// 檔案列表 + 即時索引進度 / 刪除 / 換 embedding 重建。
import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { Loader2, Trash2, Plus, RefreshCw, FileText, AlertCircle, CheckCircle2, Upload, FileCog, Library, SlidersHorizontal, Pencil, Link2 } from 'lucide-react'
import { type ModelProvider, type DocumentProcessingConfig, type KnowledgeBaseConfig } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select } from './components'
import { resolveModelInfo } from '../data/modelMatching'
import { DocumentProcessingPanel } from './DocumentProcessingPanel'
import { RetrievalPanel } from './RetrievalPanel'
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
  onKbIndex,
  type KnowledgeLibrary,
  type KnowledgeDocument,
} from '../chat/knowledgeBase'

const UPLOAD_EXTS = ['txt', 'text', 'log', 'csv', 'tsv', 'md', 'markdown', 'mdown', 'mkd', 'pdf', 'docx', 'xlsx', 'html', 'htm', 'png', 'jpg', 'jpeg', 'webp', 'bmp', 'tif', 'tiff', 'gif']

// Embedding 模型選擇器：從 provider 的 availableModels 取建議（含未啟用的 embedding
// 模型，如 bge-m3 / text-embedding-3-small），並允許自由輸入——有些 provider 不在
// /models 裡列出 embedding 模型。刻意不復用 ModelPairSelect（那個只列 enabledModels）。
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
  const t = (zh: string, en: string) => (lang === 'zh' || lang === 'zh-TW' ? zh : en)
  const enabled = providers.filter((p) => p.enabled !== false)
  const selected = enabled.find((p) => p.id === providerId)
  // Build the provider dropdown. If the bound providerId isn't among the
  // enabled providers (disabled, or deleted/never-saved → dangling binding),
  // surface it explicitly with a readable label so the Select never falls back
  // to showing the raw provider id.
  const options = [
    { value: '', label: t('選擇供應商…', 'Pick provider…') },
    ...enabled.map((p) => ({ value: p.id, label: p.name || p.id })),
  ]
  if (providerId && !options.some((o) => o.value === providerId)) {
    const known = providers.find((p) => p.id === providerId)
    const label = known
      ? `${known.name || known.id}${known.enabled === false ? t('（已停用）', ' (disabled)') : ''}`
      : t('⚠ 供應商已刪除，請重新選擇', '⚠ provider missing — re-select')
    options.unshift({ value: providerId, label })
  }

  // Model dropdown: pick from THIS provider's configured (enabled) models —
  // the exact set the user curated in provider settings ("these models appear
  // in each feature's model selector"). Do NOT pull the full /models list.
  const configuredModels = selected?.enabledModels ?? []
  const modelOptions = [
    { value: '', label: t('選擇 embedding 模型…', 'Pick embedding model…') },
    ...configuredModels.map((m) => ({ value: m, label: m })),
  ]
  // Keep an existing/legacy binding visible even if it's no longer enabled.
  if (model && !modelOptions.some((o) => o.value === model)) {
    modelOptions.push({ value: model, label: model })
  }

  // 解析模型資訊（含嵌入維度/多語言/上下文），用於展示能力徽章。
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
          {info?.multilingual && <KbInfoPill>{t('多語言', 'Multilingual')}</KbInfoPill>}
          {info?.dimensions ? <KbInfoPill>{t(`${info.dimensions} 維`, `${info.dimensions}d`)}</KbInfoPill> : null}
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

function KbPageHeader({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div>
      <h2 className="text-base font-semibold text-zinc-900 dark:text-zinc-50">{title}</h2>
      <p className="mt-1 text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">{subtitle}</p>
    </div>
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
}: {
  providers: ModelProvider[]
  lang: Lang
  docProcessing?: DocumentProcessingConfig
  onChangeDocProcessing: (next: DocumentProcessingConfig) => void
  kbConfig?: KnowledgeBaseConfig
  onChangeKbConfig: (next: KnowledgeBaseConfig) => void
}) {
  const t = (zh: string, en: string) => (lang === 'zh' || lang === 'zh-TW' ? zh : en)

  const [libraries, setLibraries] = useState<KnowledgeLibrary[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  // 右欄檢視：某個庫 / 新建 / 檔案處理 / 檢索設定
  const [rightView, setRightView] = useState<'library' | 'new' | 'docproc' | 'retrieval'>('library')
  const [docs, setDocs] = useState<KnowledgeDocument[]>([])
  const [progress, setProgress] = useState<Record<string, Progress>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 建庫表單
  const [newName, setNewName] = useState('')
  const [newProviderId, setNewProviderId] = useState('')
  const [newModel, setNewModel] = useState('')

  // 選中庫的 embedding 編輯草稿（改完點「應用並重建」才生效）
  const [editProviderId, setEditProviderId] = useState('')
  const [editModel, setEditModel] = useState('')

  // 網址匯入輸入框
  const [urlInput, setUrlInput] = useState('')
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')

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

  // 選中庫變化時，把 embedding 草稿重置為該庫當前配置。
  useEffect(() => {
    setEditProviderId(selected?.embeddingProviderId ?? '')
    setEditModel(selected?.embeddingModel ?? '')
  }, [selected?.id, selected?.embeddingProviderId, selected?.embeddingModel])

  // 即時索引進度：更新進度條；終態時重新整理檔案+庫計數。
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
      setError(t('請填寫名稱並選擇 embedding 模型', 'Enter a name and pick an embedding model'))
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
    setBusy(true)
    setError(null)
    // Try every file independently so one failure doesn't silently drop the
    // rest; report which ones failed and always refresh to reflect successes.
    const failures: string[] = []
    for (const path of paths) {
      try {
        await kbUploadDocument(selectedId, path)
      } catch (e) {
        const name = path.split(/[\\/]/).pop() || path
        failures.push(`${name}: ${e}`)
      }
    }
    await refreshDocs(selectedId).catch(() => {})
    await refreshLibraries()
    setBusy(false)
    if (failures.length > 0) {
      setError(t(`${failures.length} 個檔案匯入失敗：`, `${failures.length} file(s) failed: `) + failures.join('; '))
    }
  }

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
      setError(t('網址匯入失敗：', 'URL import failed: ') + String(e))
    } finally {
      setBusy(false)
    }
  }

  const handleDeleteLibrary = async (kbId: string) => {
    if (!confirm(t('刪除該知識庫及其所有檔案？', 'Delete this knowledge base and all its documents?'))) return
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
          '更換 embedding 模型會重建整個知識庫索引（重新呼叫 embedding，可能耗時與產生費用）。繼續？',
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
    if (!confirm(t('重建該知識庫的全部索引？', 'Rebuild the whole index for this library?'))) return
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
      {/* 左側二級側邊欄：分隔線拉滿可用高度 */}
      <nav className="relative flex h-full min-h-full w-44 shrink-0 flex-col self-stretch pr-3">
        <div
          className="pointer-events-none absolute inset-y-0 right-0 w-px bg-zinc-200/80 dark:bg-zinc-800"
          aria-hidden
        />
        <div className="space-y-0.5">
        <button
          type="button"
          onClick={() => setRightView('docproc')}
          className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition ${
            rightView === 'docproc'
              ? 'bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300'
              : 'text-zinc-700 hover:bg-zinc-100 dark:text-zinc-200 dark:hover:bg-zinc-800'
          }`}
        >
          <FileCog size={14} className="shrink-0 text-zinc-400" /> {t('檔案處理', 'Doc processing')}
        </button>

        <button
          type="button"
          onClick={() => setRightView('retrieval')}
          className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition ${
            rightView === 'retrieval'
              ? 'bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300'
              : 'text-zinc-700 hover:bg-zinc-100 dark:text-zinc-200 dark:hover:bg-zinc-800'
          }`}
        >
          <SlidersHorizontal size={14} className="shrink-0 text-zinc-400" /> {t('檢索', 'Retrieval')}
        </button>

        <div className="my-1.5 border-t border-zinc-200/70 dark:border-zinc-800" />

        <div className="px-2 pb-1 pt-1 text-[11px] font-medium uppercase tracking-wide text-zinc-400">
          {t('知識庫', 'Libraries')}
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
              <span className="shrink-0 text-xs text-zinc-400" title={t('檔案數', 'Documents')}>
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
          <Plus size={14} className="shrink-0" /> {t('新建知識庫', 'New library')}
        </button>
        </div>
      </nav>

      {/* 右側內容 */}
      <div className="min-w-0 flex-1 space-y-4 pl-4">
        {error && (
          <div className="flex items-center gap-2 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600 dark:bg-red-950/40 dark:text-red-400">
            <AlertCircle size={14} />
            <span className="flex-1">{error}</span>
            <button type="button" onClick={() => setError(null)} className="text-xs underline">
              {t('關閉', 'dismiss')}
            </button>
          </div>
        )}

        {/* 新建知識庫 */}
        {rightView === 'new' && (
          <SettingsGroup title={t('新建知識庫', 'New knowledge base')}>
            <div className="flex flex-wrap items-center gap-2 py-2">
              <Input
                value={newName}
                onChange={setNewName}
                placeholder={t('知識庫名稱', 'Library name')}
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
                <Plus size={14} /> {t('建立', 'Create')}
              </button>
            </div>
            <p className="px-1 pb-1 text-xs text-zinc-500">
              {t(
                'embedding 模型決定向量維度，建庫後更換需重建索引。需選用支援 /embeddings 介面的模型。',
                'The embedding model fixes the vector dimension; changing it later rebuilds the index. Use a model that serves the /embeddings endpoint.'
              )}
            </p>
          </SettingsGroup>
        )}

        {/* 檔案處理 */}
        {rightView === 'docproc' && (
          <div className="space-y-4">
            <KbPageHeader
              title={t('檔案處理', 'Doc processing')}
              subtitle={t(
                'Kivio 本機解析檔案，免費離線；圖片需配置 OCR 後入庫。',
                'Kivio parses documents locally — free and offline; images need OCR enabled.',
              )}
            />
            <DocumentProcessingPanel
              config={docProcessing}
              lang={lang}
              onChange={onChangeDocProcessing}
            />
          </div>
        )}

        {/* 檢索設定 */}
        {rightView === 'retrieval' && (
          <div className="space-y-4">
            <KbPageHeader
              title={t('檢索', 'Retrieval')}
              subtitle={t(
                '配置混合檢索權重與可選 rerank；僅 embedding 模型也可直接檢索。',
                'Tune hybrid weights and optional rerank; embedding alone is enough to search.',
              )}
            />
            <RetrievalPanel
              config={kbConfig}
              providers={providers}
              lang={lang}
              onChange={onChangeKbConfig}
            />
          </div>
        )}

        {/* 選中庫為空時的提示 */}
        {rightView === 'library' && !selected && (
          <p className="px-1 pt-6 text-sm text-zinc-500">
            {t(
              '從左側選擇一個知識庫，或點「新建知識庫」。',
              'Pick a library on the left, or click “New library”.'
            )}
          </p>
        )}

        {/* 選中庫詳情 */}
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
                      placeholder={t('知識庫名稱', 'Library name')}
                    />
                    <button
                      type="button"
                      disabled={busy || !renameDraft.trim()}
                      onClick={() => void handleRename(selected.id, renameDraft)}
                      className="rounded-lg bg-indigo-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
                    >
                      {t('儲存', 'Save')}
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
                    `${selected.docCount} 個檔案 · ${selected.chunkCount} 塊`,
                    `${selected.docCount} docs · ${selected.chunkCount} chunks`,
                  )}
                  {selected.embeddingDim > 0
                    ? t(` · ${selected.embeddingDim} 維`, ` · ${selected.embeddingDim}d`)
                    : ''}
                </p>
              </div>
              {renamingId !== selected.id && (
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    onClick={() => startRenaming(selected.id, selected.name)}
                    className="rounded-lg p-1.5 text-zinc-400 transition hover:bg-zinc-100 hover:text-zinc-600 dark:hover:bg-zinc-800 dark:hover:text-zinc-200"
                    title={t('重新命名', 'Rename')}
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    type="button"
                    onClick={() => handleDeleteLibrary(selected.id)}
                    className="rounded-lg p-1.5 text-zinc-400 transition hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-950/40 dark:hover:text-red-400"
                    title={t('刪除庫', 'Delete library')}
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
                    {t('應用並重建索引', 'Apply & rebuild index')}
                  </button>
                )}
              </div>
            </SettingsGroup>

            <SettingsGroup title={t('檔案', 'Documents')}>
              <div className="space-y-3 py-2">
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    disabled={busy}
                    onClick={handleUpload}
                    className="inline-flex items-center gap-1 rounded-lg bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
                  >
                    <Upload size={14} /> {t('匯入檔案', 'Add documents')}
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
                      placeholder={t('貼上網址匯入（https://…）', 'Paste URL (https://…)')}
                      mono
                    />
                  </div>
                  <button
                    type="button"
                    disabled={busy || !urlInput.trim()}
                    onClick={handleImportUrl}
                    className="inline-flex items-center gap-1 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm hover:bg-zinc-50 disabled:opacity-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
                  >
                    <Plus size={14} /> {t('匯入網址', 'Add URL')}
                  </button>
                </div>

                {docs.length === 0 ? (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={handleUpload}
                    className="flex w-full flex-col items-center gap-2 rounded-lg border border-dashed border-zinc-300 bg-zinc-50/50 px-4 py-8 text-center transition hover:border-indigo-300 hover:bg-indigo-50/40 disabled:opacity-50 dark:border-zinc-700 dark:bg-zinc-900/30 dark:hover:border-indigo-800 dark:hover:bg-indigo-950/20"
                  >
                    <Upload size={20} className="text-zinc-400" />
                    <span className="text-sm font-medium text-zinc-600 dark:text-zinc-300">
                      {t('點選匯入檔案', 'Click to add documents')}
                    </span>
                    <span className="max-w-md text-xs leading-relaxed text-zinc-400">
                      {t(
                        '支援 txt / md / pdf / docx / xlsx / html、圖片（需開啟 OCR），或使用上方網址匯入',
                        'txt, md, pdf, docx, xlsx, html, images (OCR required), or import a URL above',
                      )}
                    </span>
                  </button>
                ) : (
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
  const t = (zh: string, en: string) => (lang === 'zh' || lang === 'zh-TW' ? zh : en)
  return (
    <div className="flex items-center gap-2 px-3 py-2 text-sm">
      <FileText size={14} className="shrink-0 text-zinc-400" />
      <span className="flex-1 truncate" title={doc.name}>
        {doc.name}
      </span>
      {doc.status === 'indexing' && (
        <span className="flex items-center gap-1 text-xs text-indigo-500">
          <Loader2 size={12} className="animate-spin" />
          {progress && progress.total > 0
            ? `${progress.indexed}/${progress.total}`
            : t('索引中', 'indexing')}
        </span>
      )}
      {doc.status === 'ready' && (
        <span className="flex items-center gap-1 text-xs text-emerald-500">
          <CheckCircle2 size={12} /> {t(`${doc.chunkCount} 塊`, `${doc.chunkCount}`)}
        </span>
      )}
      {doc.status === 'error' && (
        <span className="flex items-center gap-1 text-xs text-red-500" title={doc.error ?? ''}>
          <AlertCircle size={12} /> {t('失敗', 'error')}
        </span>
      )}
      <button
        type="button"
        onClick={onDelete}
        className="shrink-0 rounded p-1 text-zinc-400 hover:bg-zinc-100 hover:text-red-500 dark:hover:bg-zinc-800"
        title={t('刪除檔案', 'Delete document')}
      >
        <Trash2 size={13} />
      </button>
    </div>
  )
}
