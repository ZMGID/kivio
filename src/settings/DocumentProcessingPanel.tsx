// 文档处理设置区（知识库页）：Kivio 内置本地解析 + 图片 OCR，
// 以及可选第三方解析服务（MinerU / LlamaParse，扫描版/复杂版面）。
import { Download, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  api,
  type DocProcessorProvider,
  type DocumentProcessingConfig,
  type OcrEngine,
  type PdfStrategy,
  type RapidOcrStatus,
} from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Select, SettingRow, Toggle, Input } from './components'
import { Button, IconButton } from '../components/Button'

const EMPTY: DocumentProcessingConfig = {
  ocrEngine: 'off',
  pdfStrategy: 'text',
  activeProcessor: '',
  fallbackToThirdParty: false,
  providers: [],
}

// 两个固定的第三方解析服务；密钥填在各自条目里。
const THIRD_PARTY: Array<{ kind: string; name: string; keyUrl: string }> = [
  { kind: 'mineru', name: 'MinerU', keyUrl: 'https://mineru.net' },
  { kind: 'llamaparse', name: 'LlamaParse', keyUrl: 'https://cloud.llamaindex.ai' },
]

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
  const cfg = { ...EMPTY, ...config }

  const patch = (updates: Partial<DocumentProcessingConfig>) => onChange({ ...cfg, ...updates })

  const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent)

  // 固定 provider 条目按 kind 定位（id = kind）；改 key 时就地建/改。
  const providerOf = (kind: string) => cfg.providers.find((p) => p.kind === kind)
  const setProviderKey = (kind: string, name: string, key: string) => {
    const others = cfg.providers.filter((p) => p.kind !== kind)
    const next: DocProcessorProvider = {
      ...(providerOf(kind) ?? { id: kind, name, kind, apiKeys: [], baseUrl: '', enabled: false }),
      apiKeys: key.trim() ? [key] : [],
      enabled: Boolean(key.trim()),
    }
    // 选中的服务密钥被清空时,回退到内置,避免留下必失败的路由。
    const updates: Partial<DocumentProcessingConfig> = { providers: [...others, next] }
    if (!key.trim() && cfg.activeProcessor === kind) updates.activeProcessor = ''
    patch(updates)
  }

  // id = kind,所以 activeProcessor 本身就是 kind('' = 内置)。
  const activeKind = cfg.activeProcessor

  return (
    <div className="space-y-4">
      <SettingsGroup title={t('文档解析服务', 'Parsing service')}>
        <div className="px-1 py-2">
          <div className="kv-seg w-full">
            {[{ kind: '', name: t('Kivio 内置', 'Kivio built-in') }, ...THIRD_PARTY].map((s) => (
              <button
                key={s.kind || 'builtin'}
                type="button"
                className={`flex-1 ${activeKind === s.kind ? 'active' : ''}`}
                onClick={() => patch({ activeProcessor: s.kind })}
              >
                {s.name}
              </button>
            ))}
          </div>
          <p className="kv-row-desc mt-1.5">
            {activeKind === ''
              ? t(
                  'Kivio 本地解析 txt / md / html / PDF 文字层 / docx / xlsx，免费离线。',
                  'Kivio parses txt, md, html, PDF text layer, docx and xlsx locally — free and offline.',
                )
              : t(
                  '文档上传到所选服务解析为 Markdown（适合扫描版 / 复杂版面），需要 API 密钥。',
                  'Documents are uploaded to the selected service and parsed to Markdown (good for scanned / complex layouts). Requires an API key.',
                )}
          </p>
        </div>

        {THIRD_PARTY.map((s) => {
          const p = providerOf(s.kind)
          const key = p?.apiKeys?.[0] ?? ''
          const needsKey = activeKind === s.kind && !key.trim()
          if (activeKind !== s.kind && !key.trim()) return null
          return (
            <SettingRow
              key={s.kind}
              label={t(`${s.name} API 密钥`, `${s.name} API key`)}
              description={t(`在 ${s.keyUrl} 获取`, `Get one at ${s.keyUrl}`)}
            >
              <Input
                type="password"
                className={`w-64 ${needsKey ? '!border-amber-400' : ''}`}
                value={key}
                onChange={(v) => setProviderKey(s.kind, s.name, v)}
                placeholder={t('粘贴密钥…', 'Paste key…')}
                mono
              />
            </SettingRow>
          )
        })}

        {activeKind === '' && cfg.providers.some((p) => p.enabled) && (
          <SettingRow
            label={t('解析失败时回退第三方', 'Fall back to third-party')}
            description={t(
              '内置抽不出文本（如扫描版 PDF）时，自动改用已配置的解析服务。',
              'When built-in extracts no text (e.g. scanned PDFs), retry with a configured service.',
            )}
          >
            <Toggle
              checked={cfg.fallbackToThirdParty}
              onChange={(v) => patch({ fallbackToThirdParty: v })}
            />
          </SettingRow>
        )}
      </SettingsGroup>

      <SettingsGroup title={t('解析选项', 'Parsing options')}>
        <SettingRow
          label={t('OCR 引擎', 'OCR engine')}
        >
          <Select
            className="w-52"
            value={cfg.ocrEngine}
            onChange={(v) => patch({ ocrEngine: v as OcrEngine })}
            options={[
              { value: 'off', label: t('关闭', 'Off') },
              { value: 'system', label: t('系统 OCR', 'System OCR') },
              { value: 'rapid_ocr', label: t('RapidOCR 离线', 'RapidOCR (offline)') },
            ]}
          />
        </SettingRow>

        {cfg.ocrEngine === 'system' && (
          <p className="kv-row-desc -mt-1 px-1 pb-1">
            {isMac
              ? t('macOS：Apple Vision', 'macOS: Apple Vision')
              : t('Windows：Windows.Media.Ocr', 'Windows: Windows.Media.Ocr')}
          </p>
        )}

        {cfg.ocrEngine === 'rapid_ocr' && <RapidOcrWidget t={t} />}

        <SettingRow
          label={t('PDF 处理策略', 'PDF strategy')}
        >
          <Select
            className="w-52"
            value={cfg.pdfStrategy}
            onChange={(v) => patch({ pdfStrategy: v as PdfStrategy })}
            options={[
              { value: 'text', label: t('文字层优先', 'Text layer') },
              { value: 'force_ocr', label: t('强制 OCR', 'Force OCR') },
            ]}
          />
        </SettingRow>

        {cfg.pdfStrategy === 'force_ocr' && (
          <p className="px-1 pb-1 text-[11px] text-amber-700 dark:text-amber-200">
            {t(
              '内置仅支持 PDF 文字层，强制 OCR（扫描版）暂未启用。',
              'Built-in only supports the PDF text layer; force OCR (scanned PDFs) is not yet enabled.',
            )}
          </p>
        )}
      </SettingsGroup>

      <p className="text-xs leading-relaxed text-zinc-400 dark:text-zinc-500">
        {t(
          '支持格式：txt / md / html / PDF（文字层）/ docx / xlsx；图片 png/jpg/webp 等需开启 OCR。',
          'Supported: txt, md, html, PDF (text layer), docx, xlsx; images (png/jpg/webp) require OCR.',
        )}
      </p>
    </div>
  )
}

/** RapidOCR 离线引擎的状态/下载组件，本地自管状态。 */
function RapidOcrWidget({ t }: { t: (zh: string, en: string) => string }) {
  const [status, setStatus] = useState<RapidOcrStatus | null>(null)
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'failed'>('idle')
  const [downloadError, setDownloadError] = useState('')

  const refresh = () => {
    api
      .rapidOcrStatus()
      .then(setStatus)
      .catch(() => setStatus({ available: false }))
  }

  useEffect(() => {
    refresh()
  }, [])

  const download = async () => {
    setDownloadState('downloading')
    setDownloadError('')
    try {
      const res = await api.rapidOcrInstall()
      if (res.success) {
        setDownloadState('idle')
        refresh()
      } else {
        setDownloadState('failed')
        setDownloadError(res.message)
      }
    } catch (e) {
      setDownloadState('failed')
      setDownloadError(String(e))
    }
  }

  const available = status?.available

  return (
    <div className="mx-1 mb-2 rounded-lg border border-zinc-200 bg-zinc-50/80 px-3 py-2.5 dark:border-zinc-700 dark:bg-zinc-900/40">
      <p className="mb-2 text-xs text-zinc-400 dark:text-zinc-500">
        {t(
          'PP-OCRv6 覆盖中英日等 50 种语言，需单独下载（约 150MB）。',
          'PP-OCRv6 covers 50 languages (CJK + Latin); downloaded separately (~150MB).',
        )}
      </p>
      {available ? (
        <div className="flex items-start gap-2">
          <span className="mt-1.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium text-zinc-700 dark:text-zinc-200">
              {t('RapidOCR 已就绪', 'RapidOCR ready')}
            </div>
            {status?.modelDir && (
              <div className="mt-0.5 break-all font-mono text-[11px] text-zinc-500">{status.modelDir}</div>
            )}
          </div>
          <IconButton size="xs" onClick={refresh} label={t('刷新', 'Refresh')}>
            <RefreshCw size={12} strokeWidth={2.25} />
          </IconButton>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="flex items-start gap-2">
            <span className="mt-1.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />
            <div className="flex-1 text-sm font-medium text-zinc-700 dark:text-zinc-200">
              {t('RapidOCR 模型未下载', 'RapidOCR models not downloaded')}
            </div>
            <IconButton
              size="xs"
              onClick={refresh}
              disabled={downloadState === 'downloading'}
              className="disabled:opacity-40"
              label={t('刷新', 'Refresh')}
            >
              <RefreshCw size={12} strokeWidth={2.25} />
            </IconButton>
          </div>

          {downloadState === 'downloading' ? (
            <div className="flex items-center gap-2 pl-3.5 text-sm text-zinc-500">
              <RefreshCw size={12} strokeWidth={2.25} className="animate-spin" />
              <span>{t('正在下载…', 'Downloading…')}</span>
            </div>
          ) : (
            <div className="pl-3.5">
              <Button variant="primary" size="sm" onClick={download}>
                <Download size={12} strokeWidth={2.5} />
                {t('下载离线模型', 'Download models')}
              </Button>
            </div>
          )}

          {downloadState === 'failed' && downloadError && (
            <div className="kv-inline-error break-words pl-3.5">
              {t('下载失败', 'Download failed')}: {downloadError}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
