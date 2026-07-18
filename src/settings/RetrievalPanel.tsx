// 知识库「检索」设置：检索模式（混合/纯向量）+ 权重 + 上下文 TopK + 重排 +
// 折叠的「高级」（候选池 / 送重排数 / 相关性阈值）。统一用 SettingsGroup idiom。
import { type ModelProvider, type KnowledgeBaseConfig } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select, SettingRow, SliderField } from './components'

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

export function RetrievalPanel({
  config,
  providers,
  lang,
  onChange,
}: {
  config?: KnowledgeBaseConfig
  providers: ModelProvider[]
  lang: Lang
  onChange: (next: KnowledgeBaseConfig) => void
}) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const cfg = config ?? DEFAULT
  const patch = (u: Partial<KnowledgeBaseConfig>) => onChange({ ...cfg, ...u })

  const enabled = providers.filter((p) => p.enabled !== false)
  const rerankProvider = enabled.find((p) => p.id === cfg.rerankProviderId)
  const rerankModels = rerankProvider?.enabledModels ?? []

  const modes = [
    { id: 'hybrid', name: t('混合', 'Hybrid') },
    { id: 'vector', name: t('纯向量', 'Vector only') },
  ] as const

  return (
    <div className="space-y-4">
      {/* 检索模式 + 权重 + 上下文数量 */}
      <SettingsGroup title={t('检索', 'Retrieval')}>
        <div className="px-1 py-2">
          <div className="kv-seg w-full">
            {modes.map((m) => (
              <button
                key={m.id}
                type="button"
                className={`flex-1 ${(cfg.hybridEnabled ? 'hybrid' : 'vector') === m.id ? 'active' : ''}`}
                onClick={() => patch({ hybridEnabled: m.id === 'hybrid' })}
              >
                {m.name}
              </button>
            ))}
          </div>
          <p className="kv-row-desc mt-1.5">
            {cfg.hybridEnabled
              ? t(
                  '向量（语义）+ 关键词（BM25）双路检索，RRF 融合。对精确词、编号和弱 embedding 更稳。',
                  'Vector (semantic) + keyword (BM25) fused via RRF. More robust for exact terms, codes and weaker embeddings.',
                )
              : t(
                  '只用向量语义检索。embedding 足够强、查询以语义为主时更简洁。',
                  'Vector semantic search only. Simpler when your embedding model is strong and queries are semantic.',
                )}
          </p>
        </div>

        {cfg.hybridEnabled && (
          <div className="grid gap-1 sm:grid-cols-2">
            <SettingRow label={t('向量权重', 'Vector weight')} stack>
              <Input
                type="number"
                className="w-full max-w-[8rem]"
                value={String(cfg.weightVector)}
                onChange={(v) => patch({ weightVector: Number(v) || 0 })}
              />
            </SettingRow>
            <SettingRow label={t('关键词权重', 'Keyword weight')} stack>
              <Input
                type="number"
                className="w-full max-w-[8rem]"
                value={String(cfg.weightKeyword)}
                onChange={(v) => patch({ weightKeyword: Number(v) || 0 })}
              />
            </SettingRow>
          </div>
        )}

        <SliderField
          label={t('上下文 TopK', 'Context TopK')}
          value={cfg.topK}
          min={1}
          max={20}
          step={1}
          onChange={(v) => patch({ topK: v })}
          hint={t('每次检索最终返回给模型的片段数量。', 'Passages finally returned to the model per search.')}
        />
      </SettingsGroup>

      {/* 重排 */}
      <SettingsGroup title={t('重排（Rerank）', 'Rerank')}>
        <SettingRow
          label={t('Rerank 提供商', 'Rerank provider')}
          description={t('留空关闭；失败时降级为融合顺序', 'Blank = off; failures use fused order')}
        >
          <Select
            className="w-52"
            value={cfg.rerankProviderId}
            onChange={(pid) => patch({ rerankProviderId: pid, rerankModel: '' })}
            options={[
              { value: '', label: t('关闭', 'Off') },
              ...enabled.map((p) => ({ value: p.id, label: p.name || p.id })),
            ]}
          />
        </SettingRow>

        {cfg.rerankProviderId && (
          <SettingRow label={t('Rerank 模型', 'Rerank model')}>
            <Select
              className="w-64"
              value={cfg.rerankModel}
              onChange={(m) => patch({ rerankModel: m })}
              options={[
                { value: '', label: t('选择 rerank 模型…', 'Pick rerank model…') },
                ...rerankModels.map((m) => ({ value: m, label: m })),
              ]}
            />
          </SettingRow>
        )}
      </SettingsGroup>

      {/* 高级：默认收起（原生 details 折叠） */}
      <details className="kv-group">
        <summary className="kv-group-title cursor-pointer select-none">
          {t('高级', 'Advanced')}
        </summary>
        <div className="mt-1 space-y-1">
          <SettingRow
            label={t('候选池大小', 'Candidate pool')}
            description={t('每库融合候选数 (20–200)，越大召回越全', 'Fused candidates per library (20–200)')}
          >
            <Input
              type="number"
              className="w-28"
              value={String(cfg.candidateK ?? 60)}
              onChange={(v) => patch({ candidateK: Math.min(200, Math.max(20, Number(v) || 60)) })}
            />
          </SettingRow>
          <SettingRow
            label={t('送重排数', 'Rerank pool')}
            description={t('送 rerank 的候选数 (5–50)，仅 rerank 开启时生效', 'Candidates sent to reranker (5–50)')}
          >
            <Input
              type="number"
              className="w-28"
              value={String(cfg.rerankTopK ?? 20)}
              onChange={(v) => patch({ rerankTopK: Math.min(50, Math.max(5, Number(v) || 20)) })}
            />
          </SettingRow>
          <SettingRow
            label={t('相关性阈值', 'Relevance threshold')}
            description={t(
              '0=关闭；rerank 开=按 relevance 分过滤，关=向量命中的余弦下限',
              '0 = off; with rerank filters by relevance score, else a cosine floor for vector-only hits',
            )}
          >
            <Input
              type="number"
              className="w-28"
              value={String(cfg.minScore ?? 0)}
              onChange={(v) => patch({ minScore: Math.min(1, Math.max(0, Number(v) || 0)) })}
            />
          </SettingRow>
        </div>
      </details>
    </div>
  )
}
