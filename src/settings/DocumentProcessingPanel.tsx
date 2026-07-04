// 檔案處理設定區（知識庫頁）：僅 Kivio 內建本機解析 + 圖片 OCR。
// 第三方處理器（MinerU/Doc2X/自訂）已掛起。
import { Download, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  api,
  type DocumentProcessingConfig,
  type OcrEngine,
  type PdfStrategy,
  type RapidOcrStatus,
} from '../api/tauri'
import { type Lang } from './i18n'
import { SettingsGroup, Select, SettingRow } from './components'

const EMPTY: DocumentProcessingConfig = {
  ocrEngine: 'off',
  pdfStrategy: 'text',
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
  const t = (zh: string, en: string) => (lang === 'zh' || lang === 'zh-TW' ? zh : en)
  const cfg = config ?? EMPTY

  const patch = (updates: Partial<DocumentProcessingConfig>) => onChange({ ...cfg, ...updates })

  const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent)

  return (
    <div className="space-y-4">
      <SettingsGroup title={t('解析選項', 'Parsing options')}>
        <SettingRow
          label={t('OCR 引擎', 'OCR engine')}
          description={t(
            '圖片入庫前識別文字；關閉則跳過圖片檔案。',
            'Recognize text in images before indexing; off skips image files.',
          )}
        >
          <Select
            className="w-52"
            value={cfg.ocrEngine}
            onChange={(v) => patch({ ocrEngine: v as OcrEngine })}
            options={[
              { value: 'off', label: t('關閉', 'Off') },
              { value: 'system', label: t('系統 OCR', 'System OCR') },
              { value: 'rapid_ocr', label: t('RapidOCR 離線', 'RapidOCR (offline)') },
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
          label={t('PDF 處理策略', 'PDF strategy')}
          description={t(
            '預設讀取 PDF 文字層；掃描版強制 OCR 暫未啟用。',
            'Reads the PDF text layer by default; force OCR for scans is not enabled yet.',
          )}
        >
          <Select
            className="w-52"
            value={cfg.pdfStrategy}
            onChange={(v) => patch({ pdfStrategy: v as PdfStrategy })}
            options={[
              { value: 'text', label: t('文字層優先', 'Text layer') },
              { value: 'force_ocr', label: t('強制 OCR', 'Force OCR') },
            ]}
          />
        </SettingRow>

        {cfg.pdfStrategy === 'force_ocr' && (
          <p className="px-1 pb-1 text-[11px] text-amber-700 dark:text-amber-200">
            {t(
              '內建僅支援 PDF 文字層，強制 OCR（掃描版）暫未啟用。',
              'Built-in only supports the PDF text layer; force OCR (scanned PDFs) is not yet enabled.',
            )}
          </p>
        )}
      </SettingsGroup>

      <p className="text-xs leading-relaxed text-zinc-400 dark:text-zinc-500">
        {t(
          '支援格式：txt / md / html / PDF（文字層）/ docx / xlsx；圖片 png/jpg/webp 等需開啟 OCR。',
          'Supported: txt, md, html, PDF (text layer), docx, xlsx; images (png/jpg/webp) require OCR.',
        )}
      </p>
    </div>
  )
}

/** RapidOCR 離線引擎的狀態/下載元件，本機自管狀態。 */
function RapidOcrWidget({ t }: { t: (zh: string, en: string) => string }) {
  const [status, setStatus] = useState<RapidOcrStatus | null>(null)
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'failed'>('idle')
  const [downloadError, setDownloadError] = useState('')

  const refresh = () => {
    api
      .rapidOcrStatus()
      .then(setStatus)
      .catch(() => setStatus({ modelsAvailable: false }))
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

  return (
    <div className="mx-1 mb-2 rounded-lg border border-zinc-200 bg-zinc-50/80 px-3 py-2.5 dark:border-zinc-700 dark:bg-zinc-900/40">
      {status?.modelsAvailable ? (
        <div className="flex items-start gap-2">
          <span className="mt-1.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium text-zinc-700 dark:text-zinc-200">
              {t('RapidOCR 已就緒', 'RapidOCR ready')}
            </div>
            {status.modelDir && (
              <div className="mt-0.5 break-all font-mono text-[11px] text-zinc-500">{status.modelDir}</div>
            )}
          </div>
          <button onClick={refresh} className="kv-icon-btn" title={t('重新整理', 'Refresh')}>
            <RefreshCw size={12} strokeWidth={2.25} />
          </button>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="flex items-start gap-2">
            <span className="mt-1.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />
            <div className="flex-1 text-sm font-medium text-zinc-700 dark:text-zinc-200">
              {t('RapidOCR 模型未下載', 'RapidOCR models not downloaded')}
            </div>
            <button
              onClick={refresh}
              disabled={downloadState === 'downloading'}
              className="kv-icon-btn disabled:opacity-40"
              title={t('重新整理', 'Refresh')}
            >
              <RefreshCw size={12} strokeWidth={2.25} />
            </button>
          </div>

          {downloadState === 'downloading' ? (
            <div className="flex items-center gap-2 pl-3.5 text-sm text-zinc-500">
              <RefreshCw size={12} strokeWidth={2.25} className="animate-spin" />
              <span>{t('正在下載…', 'Downloading…')}</span>
            </div>
          ) : (
            <div className="pl-3.5">
              <button onClick={download} className="kv-btn primary sm">
                <Download size={12} strokeWidth={2.5} />
                {t('下載離線模型', 'Download models')}
              </button>
            </div>
          )}

          {downloadState === 'failed' && downloadError && (
            <div className="kv-inline-error break-words pl-3.5">
              {t('下載失敗', 'Download failed')}: {downloadError}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
