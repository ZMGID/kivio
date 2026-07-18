// 知识库「检索」设置：hybrid(向量+关键词 RRF) 开关与权重 + 可选全局 rerank。
// 只配 embedding 即可用；hybrid 免配可关，rerank 留空即关、失败自动降级。
import { type ModelProvider, type KnowledgeBaseConfig } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select, Toggle, SettingRow } from './components'

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

  return (
    <div className="space-y-4">
      <SettingsGroup title={t('混合检索', 'Hybrid search')}>
        <SettingRow
          label={t('Hybrid 融合', 'Hybrid fusion')}
        >
          <Toggle checked={cfg.hybridEnabled} onChange={(v) => patch({ hybridEnabled: v })} />
        </SettingRow>

        {cfg.hybridEnabled && (
          <div className="grid gap-1 sm:grid-cols-2">
            <SettingRow
              label={t('向量权重', 'Vector weight')}
              stack
            >
              <Input
                type="number"
                className="w-full max-w-[8rem]"
                value={String(cfg.weightVector)}
                onChange={(v) => patch({ weightVector: Number(v) || 0 })}
              />
            </SettingRow>
            <SettingRow
              label={t('关键词权重', 'Keyword weight')}
              stack
            >
              <Input
                type="number"
                className="w-full max-w-[8rem]"
                value={String(cfg.weightKeyword)}
                onChange={(v) => patch({ weightKeyword: Number(v) || 0 })}
              />
            </SettingRow>
          </div>
        )}
      </SettingsGroup>

      <SettingsGroup title={t('候选池', 'Candidate pools')}>
        <div className="grid gap-1 sm:grid-cols-2">
          <SettingRow
            label={t('候选池大小', 'Candidate pool')}
            description={t('每库融合候选数 (20–200)', 'Fused candidates per library (20–200)')}
            stack
          >
            <Input
              type="number"
              className="w-full max-w-[8rem]"
              value={String(cfg.candidateK ?? 60)}
              onChange={(v) => patch({ candidateK: Math.min(200, Math.max(20, Number(v) || 60)) })}
            />
          </SettingRow>
          <SettingRow
            label={t('送重排数', 'Rerank pool')}
            description={t('送 rerank 的候选数 (5–50)', 'Candidates sent to reranker (5–50)')}
            stack
          >
            <Input
              type="number"
              className="w-full max-w-[8rem]"
              value={String(cfg.rerankTopK ?? 20)}
              onChange={(v) => patch({ rerankTopK: Math.min(50, Math.max(5, Number(v) || 20)) })}
            />
          </SettingRow>
        </div>
        <SettingRow
          label={t('相关性阈值', 'Relevance threshold')}
          description={t(
            '0=关闭；重排开=按 relevance 分过滤，关=向量命中的余弦相似度下限（词法命中恒过）',
            '0 = off; with rerank = filter by relevance score, else = cosine floor for vector-only hits',
          )}
          stack
        >
          <Input
            type="number"
            className="w-full max-w-[8rem]"
            value={String(cfg.minScore ?? 0)}
            onChange={(v) => patch({ minScore: Math.min(1, Math.max(0, Number(v) || 0)) })}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title={t('重排（Rerank）', 'Rerank')}>
        <SettingRow
          label={t('Rerank 提供商', 'Rerank provider')}
          description={t(
            '留空关闭；失败时降级为融合顺序',
            'Blank = off; failures use fused order',
          )}
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
    </div>
  )
}
