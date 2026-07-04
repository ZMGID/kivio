// 知識庫「檢索」設定：hybrid(向量+關鍵詞 RRF) 開關與權重 + 可選全域 rerank。
// 只配 embedding 即可用；hybrid 免配可關，rerank 留空即關、失敗自動降級。
import { type ModelProvider, type KnowledgeBaseConfig } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select, Toggle, SettingRow } from './components'

const DEFAULT: KnowledgeBaseConfig = {
  hybridEnabled: true,
  weightVector: 1,
  weightKeyword: 1,
  rerankProviderId: '',
  rerankModel: '',
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
  const t = (zh: string, en: string) => (lang.startsWith('zh') ? zh : en)
  const cfg = config ?? DEFAULT
  const patch = (u: Partial<KnowledgeBaseConfig>) => onChange({ ...cfg, ...u })

  const enabled = providers.filter((p) => p.enabled !== false)
  const rerankProvider = enabled.find((p) => p.id === cfg.rerankProviderId)
  const rerankModels = rerankProvider?.enabledModels ?? []

  return (
    <div className="space-y-4">
      <SettingsGroup title={t('混合檢索', 'Hybrid search')}>
        <SettingRow
          label={t('Hybrid 融合', 'Hybrid fusion')}
          description={t(
            '向量召回 + 關鍵詞 BM25，經 RRF 融合；對中文短詞和精確匹配更有幫助。',
            'Fuses vector recall with keyword BM25 via RRF — helps CJK short terms and exact match.',
          )}
        >
          <Toggle checked={cfg.hybridEnabled} onChange={(v) => patch({ hybridEnabled: v })} />
        </SettingRow>

        {cfg.hybridEnabled && (
          <div className="grid gap-1 sm:grid-cols-2">
            <SettingRow
              label={t('向量權重', 'Vector weight')}
              description={t('調高更偏語義相似。', 'Higher favors semantic similarity.')}
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
              label={t('關鍵詞權重', 'Keyword weight')}
              description={t('調高更偏字面匹配。', 'Higher favors literal match.')}
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

      <SettingsGroup title={t('重排（Rerank）', 'Rerank')}>
        <SettingRow
          label={t('Rerank 供應商', 'Rerank provider')}
          description={t(
            'Cohere / Jina 相容；留空關閉，呼叫失敗自動降級為融合順序。',
            'Cohere/Jina-compatible; blank = off, failures fall back to fused order.',
          )}
        >
          <Select
            className="w-52"
            value={cfg.rerankProviderId}
            onChange={(pid) => patch({ rerankProviderId: pid, rerankModel: '' })}
            options={[
              { value: '', label: t('關閉', 'Off') },
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
                { value: '', label: t('選擇 rerank 模型…', 'Pick rerank model…') },
                ...rerankModels.map((m) => ({ value: m, label: m })),
              ]}
            />
          </SettingRow>
        )}
      </SettingsGroup>
    </div>
  )
}
