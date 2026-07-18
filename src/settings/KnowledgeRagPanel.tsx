// 知识库（RAG）统一设置页：顶部启用开关 + 说明横幅 + 文档处理（OCR/PDF）+
// 文档分块滑杆 + 检索（hybrid/rerank）+ 上下文 TopK 滑杆。
// 取代原先左侧「文档处理」「检索」两个独立页面。
import { FileSearch, FileCog, Layers, SlidersHorizontal, Zap } from 'lucide-react'
import {
  type ModelProvider,
  type DocumentProcessingConfig,
  type KnowledgeBaseConfig,
} from '../api/tauri'
import { type Lang } from './i18n'
import { Toggle } from './components'
import { DocumentProcessingPanel } from './DocumentProcessingPanel'
import { RetrievalPanel } from './RetrievalPanel'

const DEFAULT: KnowledgeBaseConfig = {
  hybridEnabled: true,
  weightVector: 1,
  weightKeyword: 1,
  rerankProviderId: '',
  rerankModel: '',
  chunkTokens: 480,
  topK: 5,
  candidateK: 60,
  rerankTopK: 20,
  minScore: 0,
}

function SectionTitle({ icon, children }: { icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 text-sm font-semibold text-zinc-800 dark:text-zinc-100">
      <span className="text-indigo-500">{icon}</span>
      {children}
    </div>
  )
}

/** 带数值徽章 + min/max 刻度的滑杆卡片行。 */
function RangeCard({
  label,
  value,
  min,
  max,
  step,
  onChange,
  hint,
}: {
  label: string
  value: number
  min: number
  max: number
  step: number
  onChange: (v: number) => void
  hint?: string
}) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-zinc-50/60 px-4 py-3 dark:border-zinc-700 dark:bg-zinc-900/40">
      <div className="flex items-center justify-between gap-3">
        <span className="text-sm text-zinc-700 dark:text-zinc-200">{label}</span>
        <span className="rounded-md border border-zinc-200 bg-white px-2 py-0.5 font-mono text-xs text-zinc-700 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-200">
          {value}
        </span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-2 w-full"
        style={{ accentColor: 'var(--accent)' }}
      />
      <div className="flex justify-between text-[11px] text-zinc-400">
        <span>{min}</span>
        <span>{max}</span>
      </div>
      {hint && <p className="mt-1.5 text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">{hint}</p>}
    </div>
  )
}

export function KnowledgeRagPanel({
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
  /** knowledge_search 工具开关（chatTools.nativeTools.knowledgeSearch）。 */
  ragEnabled: boolean
  onToggleRag: (v: boolean) => void
}) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const cfg: KnowledgeBaseConfig = { ...DEFAULT, ...kbConfig }
  const patch = (u: Partial<KnowledgeBaseConfig>) => onChangeKbConfig({ ...cfg, ...u })

  return (
    <div className="space-y-5">
      {/* 页头：标题 + 启用开关 */}
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <FileSearch size={18} className="text-indigo-500" />
          <h2 className="text-base font-semibold text-zinc-900 dark:text-zinc-50">
            {t('知识库（RAG）', 'Knowledge base (RAG)')}
          </h2>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-xs text-zinc-500 dark:text-zinc-400">
            {ragEnabled ? t('已启用', 'Enabled') : t('已停用', 'Disabled')}
          </span>
          <Toggle checked={ragEnabled} onChange={onToggleRag} />
        </div>
      </div>

      {/* 说明横幅 */}
      <div className="flex items-start gap-3 rounded-xl border border-indigo-100 bg-indigo-50/70 px-4 py-3 dark:border-indigo-900/50 dark:bg-indigo-950/30">
        <Zap size={16} className="mt-0.5 shrink-0 text-indigo-500" />
        <div className="min-w-0">
          <div className="text-sm font-medium text-indigo-700 dark:text-indigo-300">
            {t('RAG 支持', 'RAG support')}
          </div>
          <p className="mt-0.5 text-xs leading-relaxed text-indigo-700/80 dark:text-indigo-300/80">
            {t(
              'RAG（检索增强生成）允许 AI 检索你导入的私有文档，以提供更准确的回复。Kivio 在本机解析、分块并建立向量索引，数据不出本机。',
              'RAG (retrieval-augmented generation) lets the AI search your own documents for grounded answers. Kivio parses, chunks and indexes everything locally — data never leaves your machine.',
            )}
          </p>
        </div>
      </div>

      {/* 文档处理 */}
      <div className="space-y-3">
        <SectionTitle icon={<FileCog size={15} />}>{t('文档处理', 'Document processing')}</SectionTitle>
        <DocumentProcessingPanel config={docProcessing} lang={lang} onChange={onChangeDocProcessing} />
      </div>

      {/* 文档分块 */}
      <div className="space-y-3">
        <SectionTitle icon={<Layers size={15} />}>{t('文档分块', 'Chunking')}</SectionTitle>
        <RangeCard
          label={t('分块大小（Tokens）', 'Chunk size (tokens)')}
          value={cfg.chunkTokens}
          min={256}
          max={8192}
          step={32}
          onChange={(v) => patch({ chunkTokens: v })}
          hint={t(
            '决定存入数据库的文本片段大小。较小的分块（如 512）检索更精确，较大的分块（如 2048）每条结果包含更多上下文。仅影响之后导入或重建索引的文档。',
            'Controls the size of text pieces stored in the index. Smaller chunks (~512) retrieve more precisely; larger ones (~2048) carry more context per hit. Applies to documents imported or reindexed from now on.',
          )}
        />
      </div>

      {/* 检索 */}
      <div className="space-y-3">
        <SectionTitle icon={<SlidersHorizontal size={15} />}>{t('检索', 'Retrieval')}</SectionTitle>
        <RetrievalPanel config={cfg} providers={providers} lang={lang} onChange={onChangeKbConfig} />
        <RangeCard
          label={t('上下文 TopK', 'Context TopK')}
          value={cfg.topK}
          min={1}
          max={20}
          step={1}
          onChange={(v) => patch({ topK: v })}
          hint={t(
            '每次检索默认返回的片段数量。',
            'Default number of passages returned per search.',
          )}
        />
      </div>
    </div>
  )
}
