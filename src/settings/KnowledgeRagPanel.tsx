// 知识库（RAG）统一设置页：启用开关 + 说明横幅 + 文档处理 + 分块 + 检索（含重排/高级）。
// 全页统一到 SettingsGroup 卡片 idiom；高级检索旋钮收进 RetrievalPanel 的折叠区。
import { FileSearch, Zap } from 'lucide-react'
import {
  type ModelProvider,
  type DocumentProcessingConfig,
  type KnowledgeBaseConfig,
} from '../api/tauri'
import { type Lang } from './i18n'
import { Toggle, SettingsGroup, SliderField } from './components'
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
    <div className="space-y-4">
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
        <p className="min-w-0 text-xs leading-relaxed text-indigo-700/90 dark:text-indigo-300/85">
          {t(
            'RAG 允许 AI 检索你导入的私有文档以提供更准确的回复。Kivio 在本机解析、分块并建立索引，数据不出本机。',
            'RAG lets the AI search your own documents for grounded answers. Kivio parses, chunks and indexes everything locally — data never leaves your machine.',
          )}
        </p>
      </div>

      {/* 文档处理（解析服务 + OCR + PDF，自带分组标题） */}
      <DocumentProcessingPanel config={docProcessing} lang={lang} onChange={onChangeDocProcessing} />

      {/* 索引 / 分块 */}
      <SettingsGroup title={t('索引', 'Indexing')}>
        <SliderField
          label={t('分块大小（Tokens）', 'Chunk size (tokens)')}
          value={cfg.chunkTokens}
          min={256}
          max={8192}
          step={32}
          onChange={(v) => patch({ chunkTokens: v })}
          hint={t(
            '存入索引的文本片段大小。较小（~512）检索更精确，较大（~2048）每条含更多上下文。仅影响之后导入或重建的文档。',
            'Size of text pieces stored in the index. Smaller (~512) retrieves more precisely; larger (~2048) carries more context. Applies to documents imported or reindexed from now on.',
          )}
        />
      </SettingsGroup>

      {/* 检索（模式 / 权重 / TopK / 重排 / 高级） */}
      <RetrievalPanel config={cfg} providers={providers} lang={lang} onChange={onChangeKbConfig} />
    </div>
  )
}
