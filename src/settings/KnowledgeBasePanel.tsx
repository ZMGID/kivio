// 知识库管理面板（Settings 页）：建库 / 选 embedding 模型 / 拖拽或选文件导入 /
// 文档列表 + 实时索引进度 / 删除 / 换 embedding 重建。
import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { Loader2, Trash2, Plus, RefreshCw, FileText, AlertCircle, CheckCircle2, Upload } from 'lucide-react'
import { type ModelProvider } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select } from './components'
import { resolveModelInfo } from '../data/modelMatching'
import {
  kbListLibraries,
  kbCreateLibrary,
  kbDeleteLibrary,
  kbRenameLibrary,
  kbListDocuments,
  kbDeleteDocument,
  kbUploadDocument,
  kbReindexLibrary,
  kbUpdateEmbedding,
  onKbIndex,
  type KnowledgeLibrary,
  type KnowledgeDocument,
} from '../chat/knowledgeBase'

const UPLOAD_EXTS = ['txt', 'text', 'log', 'csv', 'tsv', 'md', 'markdown', 'mdown', 'pdf']

// Embedding 模型选择器：从 provider 的 availableModels 取建议（含未启用的 embedding
// 模型，如 bge-m3 / text-embedding-3-small），并允许自由输入——有些 provider 不在
// /models 里列出 embedding 模型。刻意不复用 ModelPairSelect（那个只列 enabledModels）。
function EmbeddingModelPicker({
  providers,
  providerId,
  model,
  onChange,
  lang,
}: {
  providers: ModelProvider[]
  providerId: string
  model: string
  onChange: (providerId: string, model: string) => void
  lang: Lang
}) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const enabled = providers.filter((p) => p.enabled !== false)
  const selected = enabled.find((p) => p.id === providerId)
  const suggestions = selected
    ? (selected.availableModels?.length ? selected.availableModels : selected.enabledModels) ?? []
    : []
  const listId = `kb-emb-models-${providerId || 'none'}`
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
          options={[
            { value: '', label: t('选择提供商…', 'Pick provider…') },
            ...enabled.map((p) => ({ value: p.id, label: p.name })),
          ]}
        />
        <Input
          className="w-56"
          value={model}
          onChange={(m) => onChange(providerId, m)}
          placeholder={t('embedding 模型名', 'embedding model id')}
          list={listId}
          mono
        />
        <datalist id={listId}>
          {suggestions.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
      </div>
      {isEmbedding && (
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

export function KnowledgeBasePanel({ providers, lang }: { providers: ModelProvider[]; lang: Lang }) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)

  const [libraries, setLibraries] = useState<KnowledgeLibrary[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
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
  }, [selected?.id, selected?.embeddingProviderId, selected?.embeddingModel])

  // 实时索引进度：更新进度条；终态时刷新文档+库计数。
  const selectedRef = useRef<string | null>(null)
  selectedRef.current = selectedId
  useEffect(() => {
    let unlisten: (() => void) | undefined
    void onKbIndex((ev) => {
      setProgress((p) => ({ ...p, [ev.docId]: { indexed: ev.indexed, total: ev.total } }))
      if (ev.status !== 'indexing') {
        if (ev.kbId === selectedRef.current) void refreshDocs(ev.kbId)
        void refreshLibraries()
      }
    }).then((fn) => {
      unlisten = fn
    })
    return () => unlisten?.()
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
    try {
      for (const path of paths) {
        await kbUploadDocument(selectedId, path)
      }
      await refreshDocs(selectedId)
    } catch (e) {
      setError(String(e))
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

  const handleRename = async (kbId: string, current: string) => {
    const name = prompt(t('重命名知识库', 'Rename knowledge base'), current)
    if (!name || name.trim() === current) return
    try {
      await kbRenameLibrary(kbId, name.trim())
      await refreshLibraries()
    } catch (e) {
      setError(String(e))
    }
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
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      {error && (
        <div className="flex items-center gap-2 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600 dark:bg-red-950/40 dark:text-red-400">
          <AlertCircle size={14} />
          <span className="flex-1">{error}</span>
          <button type="button" onClick={() => setError(null)} className="text-xs underline">
            {t('关闭', 'dismiss')}
          </button>
        </div>
      )}

      {/* 建库 */}
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

      {libraries.length === 0 ? (
        <p className="px-1 text-sm text-zinc-500">
          {t('还没有知识库。创建一个并导入文档后，即可在对话中检索。', 'No knowledge bases yet. Create one and add documents to search them in chat.')}
        </p>
      ) : (
        <SettingsGroup title={t('知识库', 'Libraries')}>
          <div className="flex flex-wrap gap-2 py-2">
            {libraries.map((lib) => (
              <button
                key={lib.id}
                type="button"
                onClick={() => setSelectedId(lib.id)}
                className={`group flex items-center gap-2 rounded-lg border px-3 py-1.5 text-sm transition ${
                  lib.id === selectedId
                    ? 'border-indigo-500 bg-indigo-50 text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300'
                    : 'border-zinc-200 hover:border-zinc-300 dark:border-zinc-700'
                }`}
              >
                <span className="font-medium">{lib.name}</span>
                <span className="text-xs text-zinc-400">{lib.docCount}</span>
              </button>
            ))}
          </div>
        </SettingsGroup>
      )}

      {/* 选中库详情 */}
      {selected && (
        <SettingsGroup title={selected.name}>
          <div className="space-y-3 py-2">
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <span className="text-zinc-500">{t('Embedding：', 'Embedding: ')}</span>
              <EmbeddingModelPicker
                providers={providers}
                providerId={editProviderId}
                model={editModel}
                onChange={(p, m) => {
                  setEditProviderId(p)
                  setEditModel(m)
                }}
                lang={lang}
              />
              {(editProviderId !== selected.embeddingProviderId || editModel !== selected.embeddingModel) && (
                <button
                  type="button"
                  disabled={busy || !editProviderId || !editModel}
                  onClick={() => handleChangeEmbedding(editProviderId, editModel)}
                  className="rounded-lg bg-amber-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-amber-700 disabled:opacity-50"
                >
                  {t('应用并重建', 'Apply & rebuild')}
                </button>
              )}
              <span className="text-xs text-zinc-400">
                {selected.embeddingDim > 0
                  ? t(`${selected.embeddingDim} 维 · ${selected.chunkCount} 块`, `${selected.embeddingDim}d · ${selected.chunkCount} chunks`)
                  : t('尚未索引', 'not indexed yet')}
              </span>
            </div>

            <div className="flex flex-wrap gap-2">
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
              <button
                type="button"
                onClick={() => handleRename(selected.id, selected.name)}
                className="rounded-lg border border-zinc-300 px-3 py-1.5 text-sm hover:bg-zinc-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
              >
                {t('重命名', 'Rename')}
              </button>
              <button
                type="button"
                onClick={() => handleDeleteLibrary(selected.id)}
                className="inline-flex items-center gap-1 rounded-lg border border-red-300 px-3 py-1.5 text-sm text-red-600 hover:bg-red-50 dark:border-red-900 dark:hover:bg-red-950/40"
              >
                <Trash2 size={14} /> {t('删除库', 'Delete')}
              </button>
            </div>

            {/* 文档列表 */}
            <div className="divide-y divide-zinc-100 rounded-lg border border-zinc-200 dark:divide-zinc-800 dark:border-zinc-700">
              {docs.length === 0 ? (
                <p className="px-3 py-4 text-center text-sm text-zinc-400">
                  {t('暂无文档。支持 txt / md / pdf。', 'No documents. Supports txt / md / pdf.')}
                </p>
              ) : (
                docs.map((doc) => (
                  <DocRow
                    key={doc.id}
                    doc={doc}
                    progress={progress[doc.id]}
                    lang={lang}
                    onDelete={() => handleDeleteDoc(doc.id)}
                  />
                ))
              )}
            </div>
          </div>
        </SettingsGroup>
      )}
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
  )
}
