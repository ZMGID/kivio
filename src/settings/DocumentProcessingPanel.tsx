// 文档处理设置区（知识库页）：内置 Rust 解析 + 可选第三方处理器（MinerU/Doc2X/自定义）。
// 仅前端 + 配置持久化；后端处理逻辑待接入。
import { Plus, Trash2, FileCog, ServerCog, Info } from 'lucide-react'
import { type DocProcessorKind, type DocProcessorProvider, type DocumentProcessingConfig } from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Input, Select, Toggle } from './components'

const EMPTY: DocumentProcessingConfig = {
  activeProcessor: '',
  fallbackToThirdParty: false,
  providers: [],
}

const KIND_LABELS: Record<DocProcessorKind, string> = {
  mineru: 'MinerU',
  doc2x: 'Doc2X',
  custom: '自定义 / Custom',
}

function genId(): string {
  return `dp_${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

export function DocumentProcessingPanel({
  config,
  lang,
  onChange,
}: {
  config?: DocumentProcessingConfig
  lang: Lang
  onChange: (next: DocumentProcessingConfig) => void
}) {
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const cfg = config ?? EMPTY

  const patch = (updates: Partial<DocumentProcessingConfig>) => onChange({ ...cfg, ...updates })

  const addProvider = (kind: DocProcessorKind) => {
    const provider: DocProcessorProvider = {
      id: genId(),
      name: KIND_LABELS[kind],
      kind,
      apiKeys: [''],
      baseUrl: '',
      enabled: true,
    }
    patch({ providers: [...cfg.providers, provider] })
  }

  const updateProvider = (id: string, updates: Partial<DocProcessorProvider>) => {
    patch({
      providers: cfg.providers.map((p) => (p.id === id ? { ...p, ...updates } : p)),
    })
  }

  const removeProvider = (id: string) => {
    patch({
      providers: cfg.providers.filter((p) => p.id !== id),
      // 删除的若是激活处理器，回退到内置
      activeProcessor: cfg.activeProcessor === id ? '' : cfg.activeProcessor,
    })
  }

  const enabledThirdParty = cfg.providers.filter((p) => p.enabled)

  return (
    <SettingsGroup title={t('文档处理', 'Document processing')}>
      <p className="flex items-start gap-1.5 px-1 py-1 text-xs text-zinc-500">
        <Info size={13} className="mt-0.5 shrink-0" />
        <span>
          {t(
            '内置处理用 Kivio 本地解析（txt/md/html、PDF 文字层、docx/xlsx），免费、离线、批量友好。复杂版式/扫描件/公式可交给第三方文档处理服务转成 Markdown。',
            'Built-in uses Kivio local parsing (txt/md/html, PDF text layer, docx/xlsx) — free, offline, batch-friendly. Complex layouts / scans / formulas can be handed to a third-party document service that returns Markdown.',
          )}
        </span>
      </p>

      {/* 激活处理器 */}
      <div className="flex flex-wrap items-center gap-2 py-2">
        <span className="text-sm text-zinc-500">{t('使用处理器', 'Active processor')}</span>
        <Select
          className="w-56"
          value={cfg.activeProcessor}
          onChange={(v) => patch({ activeProcessor: v })}
          options={[
            { value: '', label: t('Kivio 内置（本地）', 'Kivio built-in (local)') },
            ...enabledThirdParty.map((p) => ({
              value: p.id,
              label: `${p.name}（${KIND_LABELS[p.kind]}）`,
            })),
          ]}
        />
      </div>

      {cfg.activeProcessor === '' && enabledThirdParty.length > 0 && (
        <label className="flex cursor-pointer items-center gap-2 px-1 py-1 text-sm text-zinc-600 dark:text-zinc-300">
          <Toggle
            checked={cfg.fallbackToThirdParty}
            onChange={(v) => patch({ fallbackToThirdParty: v })}
          />
          {t(
            '内置抽不出文本（如扫描件）时，回退到第三方处理器',
            'Fall back to a third-party processor when built-in extracts no text (e.g. scans)',
          )}
        </label>
      )}

      {/* 第三方处理器列表 */}
      <div className="space-y-2 py-2">
        {cfg.providers.map((p) => (
          <div
            key={p.id}
            className="rounded-lg border border-zinc-200 p-2.5 dark:border-zinc-700"
          >
            <div className="flex items-center gap-2">
              <ServerCog size={14} className="shrink-0 text-zinc-400" />
              <Input
                className="w-40"
                value={p.name}
                onChange={(v) => updateProvider(p.id, { name: v })}
                placeholder={t('名称', 'Name')}
              />
              <Select
                className="w-32"
                value={p.kind}
                onChange={(v) => updateProvider(p.id, { kind: v as DocProcessorKind })}
                options={(Object.keys(KIND_LABELS) as DocProcessorKind[]).map((k) => ({
                  value: k,
                  label: KIND_LABELS[k],
                }))}
              />
              <label className="ml-auto flex items-center gap-1.5 text-xs text-zinc-500">
                {t('启用', 'Enabled')}
                <Toggle checked={p.enabled} onChange={(v) => updateProvider(p.id, { enabled: v })} />
              </label>
              <button
                type="button"
                onClick={() => removeProvider(p.id)}
                className="shrink-0 rounded p-1 text-zinc-400 hover:bg-zinc-100 hover:text-red-500 dark:hover:bg-zinc-800"
                title={t('删除', 'Delete')}
              >
                <Trash2 size={14} />
              </button>
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Input
                className="w-72"
                value={p.baseUrl ?? ''}
                onChange={(v) => updateProvider(p.id, { baseUrl: v })}
                placeholder={t('接口地址（可留空用默认）', 'API base URL (blank = default)')}
                mono
              />
              <Input
                className="w-72"
                type="password"
                value={p.apiKeys[0] ?? ''}
                onChange={(v) => updateProvider(p.id, { apiKeys: [v] })}
                placeholder={t('API 密钥', 'API key')}
                mono
              />
            </div>
          </div>
        ))}
      </div>

      <div className="flex flex-wrap items-center gap-2 py-1">
        <span className="text-xs text-zinc-500">{t('添加处理器：', 'Add processor:')}</span>
        {(Object.keys(KIND_LABELS) as DocProcessorKind[]).map((k) => (
          <button
            key={k}
            type="button"
            onClick={() => addProvider(k)}
            className="inline-flex items-center gap-1 rounded-lg border border-zinc-300 px-2.5 py-1 text-xs hover:bg-zinc-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
          >
            <Plus size={12} /> {KIND_LABELS[k]}
          </button>
        ))}
      </div>

      <p className="flex items-center gap-1.5 rounded-md bg-amber-50 px-2 py-1 text-[11px] text-amber-700 dark:bg-amber-400/10 dark:text-amber-200">
        <FileCog size={12} className="shrink-0" />
        {t(
          'MinerU / Doc2X 走各自的异步云端 API（提交→轮询→取 Markdown），请填写有效密钥后用一个文档实测确认。自定义处理器约定：原始字节 POST 到接口地址，返回 Markdown 文本或 JSON。',
          'MinerU / Doc2X use their async cloud APIs (submit → poll → fetch Markdown); add a valid key and test with one document to confirm. Custom processor contract: POST raw bytes to the URL, receive Markdown text or JSON.',
        )}
      </p>
    </SettingsGroup>
  )
}
