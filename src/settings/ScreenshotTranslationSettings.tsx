import { Download, Pencil, RefreshCw, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { listen } from '@tauri-apps/api/event'
import { type DefaultPromptTemplates, type OfflineModelProgress, type RapidOcrStatus, type RapidOcrTier, type ReplaceTranslationPackStatus, type Settings } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { formatBytes } from '../utils/formatBytes'
import { ModelEffortSelect, ModelPairSelect } from './ModelPairSelect'
import {
  Input,
  Select,
  SettingRow,
  SettingsGroup,
  TextArea,
  Toggle,
} from './components'
import { type I18n } from '../components/i18n'

type ScreenshotTranslation = Settings['screenshotTranslation']
type RapidOcrDownloadState = 'idle' | 'downloading' | 'failed'
type ReplacePackDownloadState = 'idle' | 'downloading' | 'failed'

interface ScreenshotTranslationSettingsProps {
  settings: Settings
  isMac: boolean
  hasSystemOcr: boolean
  defaultPrompts: DefaultPromptTemplates | null
  rapidOcrStatus: RapidOcrStatus | null
  rapidOcrDownloadState: RapidOcrDownloadState
  rapidOcrDownloadError: string
  replacePackStatus: ReplaceTranslationPackStatus | null
  replacePackDownloadState: ReplacePackDownloadState
  replacePackDownloadError: string
  replacePackProgress: OfflineModelProgress | null
  t: I18n
  onUpdate: (updates: Partial<ScreenshotTranslation>) => void
  onRefreshRapidOcrStatus: () => void
  onDownloadRapidOcr: (tier: RapidOcrTier) => void
  onRefreshReplacePack: (tier: RapidOcrTier) => void
  onDownloadReplacePack: (tier: RapidOcrTier) => void
}

export function ScreenshotTranslationSettings({
  settings,
  isMac,
  hasSystemOcr,
  defaultPrompts,
  rapidOcrStatus,
  rapidOcrDownloadState,
  rapidOcrDownloadError,
  replacePackStatus,
  replacePackDownloadState,
  replacePackDownloadError,
  replacePackProgress,
  t,
  onUpdate,
  onRefreshRapidOcrStatus,
  onDownloadRapidOcr,
  onRefreshReplacePack,
  onDownloadReplacePack,
}: ScreenshotTranslationSettingsProps) {
  const screenshot = settings.screenshotTranslation
  const ocrMode = screenshot.ocrMode ?? 'cloud_vision'
  const cardWidth = screenshot.cardWidth ?? 480
  const [widthDraft, setWidthDraft] = useState(String(cardWidth))
  // 外部改宽（翻译卡右下角拖拽缩放写回设置）时同步草稿，保持设置页与卡片联动。
  useEffect(() => {
    setWidthDraft(String(cardWidth))
  }, [cardWidth])
  // Lens 拖拽缩放会经 set_translate_card_size 落盘并广播；设置页在开着时同步草稿里的宽度，
  // 否则随后一次全量 save_settings 会用陈旧草稿把这次拖拽覆盖掉。
  useEffect(() => {
    const un = listen<number>('translate-card-width', (e) => {
      if (typeof e.payload === 'number' && e.payload !== cardWidth) onUpdate({ cardWidth: e.payload })
    })
    return () => { void un.then((f) => f()) }
  }, [cardWidth, onUpdate])
  // 边打字不 clamp（避免输 "5" 立刻跳 360）；失焦/回车时 clamp 到 360–720 再提交。
  const commitCardWidth = () => {
    const n = parseInt(widthDraft, 10)
    const next = Number.isFinite(n) ? Math.max(360, Math.min(720, n)) : cardWidth
    setWidthDraft(String(next))
    if (next !== cardWidth) onUpdate({ cardWidth: next })
  }

  return (
    <>
      <SettingsGroup title={t.tabScreenshot}>
          <SettingRow label={t.enabled}>
            <Toggle
              checked={screenshot.enabled}
              onChange={(enabled) => onUpdate({ enabled })}
            />
          </SettingRow>

          {screenshot.enabled && (
            <>
              <SettingRow label={t.sectionModel}>
                <div className="flex flex-wrap items-center justify-end gap-2">
                  <ModelPairSelect
                    providerId={screenshot.providerId}
                    model={screenshot.model}
                    providers={settings.providers}
                    onChange={(providerId, model) => onUpdate({ providerId, model, thinkingLevel: '' })}
                  />
                  <ModelEffortSelect
                    providerId={screenshot.providerId}
                    model={screenshot.model}
                    providers={settings.providers}
                    value={screenshot.thinkingLevel ?? (screenshot.thinkingEnabled ? '' : 'off')}
                    onChange={(thinkingLevel) => onUpdate({ thinkingLevel })}
                  />
                </div>
              </SettingRow>

              {hasSystemOcr && (
                <SettingRow label={t.ocrEngine} description={t.ocrEngineHint}>
                  <Select
                    value={ocrMode}
                    onChange={(value) =>
                      onUpdate({
                        ocrMode: value as ScreenshotTranslation['ocrMode'],
                      })
                    }
                    options={[
                      { value: 'cloud_vision', label: t.ocrEngineCloudVision },
                      { value: 'system', label: t.ocrEngineSystem },
                      { value: 'rapid_ocr', label: t.ocrEngineRapidOcr },
                    ]}
                    className="kv-ctl"
                  />
                </SettingRow>
              )}

              {hasSystemOcr && ocrMode === 'system' && (
                <div className="kv-row-stack">
                  <div className="kv-panel">
                    <div className="kv-panel-body">
                      {isMac ? t.ocrEngineMacHint : t.ocrEngineWindowsHint}
                    </div>
                  </div>
                </div>
              )}

              {hasSystemOcr && ocrMode === 'rapid_ocr' && (
                <div className="kv-row-stack">
                  <p className="kv-row-desc !mt-0 mb-2">{t.ocrEngineRapidOcrSharedNote}</p>
                  <RapidOcrStatusPanel
                    status={rapidOcrStatus}
                    tier={screenshot?.rapidOcrTier ?? 'standard'}
                    downloadState={rapidOcrDownloadState}
                    downloadError={rapidOcrDownloadError}
                    t={t}
                    onRefresh={onRefreshRapidOcrStatus}
                    onDownload={onDownloadRapidOcr}
                    onChangeTier={(rapidOcrTier) => onUpdate({ rapidOcrTier })}
                  />
                </div>
              )}

              <SettingRow
                label={t.screenshotShowOriginal}
                description={t.screenshotShowOriginalHint}
              >
                <Toggle
                  checked={!(screenshot.directTranslate ?? false)}
                  onChange={(showOriginal) => onUpdate({ directTranslate: !showOriginal })}
                />
              </SettingRow>

              <SettingRow label={t.lensKeepFullscreen} description={t.lensKeepFullscreenHint}>
                <Toggle
                  checked={screenshot?.keepFullscreenAfterCapture !== false}
                  onChange={(keepFullscreenAfterCapture) => onUpdate({ keepFullscreenAfterCapture })}
                />
              </SettingRow>
              <SettingRow label={t.translateCardWidth} description={t.translateCardWidthHint}>
                <Input
                  type="number"
                  min={360}
                  max={720}
                  step={10}
                  className="kv-ctl-num"
                  value={widthDraft}
                  onChange={setWidthDraft}
                  onBlur={commitCardWidth}
                  onKeyDown={(e) => { if (e.key === 'Enter') commitCardWidth() }}
                />
              </SettingRow>
            </>
          )}
      </SettingsGroup>

      {screenshot.enabled && (
        <SettingsGroup title={t.replaceTranslate}>
          <SettingRow label={t.replaceTranslateEnabled}>
            <Toggle
              checked={screenshot?.replaceEnabled !== false}
              onChange={(replaceEnabled) => onUpdate({ replaceEnabled })}
            />
          </SettingRow>

          {hasSystemOcr && screenshot?.replaceEnabled !== false && (
            <SettingRow label={t.replaceTranslateOfflinePack} stack>
              <ReplaceTranslationPackPanel
                status={replacePackStatus}
                tier={screenshot?.rapidOcrTier ?? 'standard'}
                downloadState={replacePackDownloadState}
                downloadError={replacePackDownloadError}
                progress={replacePackProgress}
                t={t}
                onRefresh={onRefreshReplacePack}
                onDownload={onDownloadReplacePack}
                onChangeTier={(rapidOcrTier) => onUpdate({ rapidOcrTier })}
              />
            </SettingRow>
          )}
        </SettingsGroup>
      )}

      {screenshot.enabled && (
        <>
          <SettingsGroup title={t.sectionPrompt}>
              <PromptField
                label={t.screenshotTranslationPrompt}
                description={t.screenshotTranslationPromptHint}
                value={screenshot?.prompt || ''}
                defaultText={defaultPrompts?.screenshotTranslationTemplate || ''}
                t={t}
                onChange={(prompt) => onUpdate({ prompt })}
              />
              <PromptField
                label={t.selectedTextTranslationPrompt}
                description={t.selectedTextTranslationPromptHint}
                value={screenshot?.textPrompt || ''}
                defaultText={defaultPrompts?.selectedTextTranslationTemplate || ''}
                t={t}
                onChange={(textPrompt) => onUpdate({ textPrompt })}
              />
              <PromptField
                label={t.replaceTranslationPrompt}
                description={t.replaceTranslationPromptHint}
                value={screenshot?.replacePrompt || ''}
                defaultText={defaultPrompts?.replaceTranslationTemplate || ''}
                t={t}
                onChange={(replacePrompt) => onUpdate({ replacePrompt })}
              />
          </SettingsGroup>
        </>
      )}
    </>
  )
}

// standard/high 档位选择：替换翻译离线包面板和 RapidOCR 引擎面板绑定同一个
// rapidOcrTier 设置且可能同时可见，共用一个 Select 避免两份拷贝漂移。
function TierSelect({
  tier,
  t,
  onChangeTier,
}: {
  tier: RapidOcrTier
  t: I18n
  onChangeTier: (tier: RapidOcrTier) => void
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="kv-panel-title !mb-0">{t.rapidOcrTier}</span>
      <Select
        value={tier}
        onChange={(value) => onChangeTier(value as RapidOcrTier)}
        options={[
          { value: 'standard', label: t.rapidOcrTierStandard },
          { value: 'high', label: t.rapidOcrTierHigh },
        ]}
        className="kv-ctl"
      />
    </div>
  )
}

function ReplaceTranslationPackPanel({
  status,
  tier,
  downloadState,
  downloadError,
  progress,
  t,
  onRefresh,
  onDownload,
  onChangeTier,
}: {
  status: ReplaceTranslationPackStatus | null
  tier: RapidOcrTier
  downloadState: ReplacePackDownloadState
  downloadError: string
  progress: OfflineModelProgress | null
  t: I18n
  onRefresh: (tier: RapidOcrTier) => void
  onDownload: (tier: RapidOcrTier) => void
  onChangeTier: (tier: RapidOcrTier) => void
}) {
  const total = progress?.overallTotalBytes || status?.totalBytes || 0
  const downloaded = progress?.overallDownloadedBytes || status?.readyBytes || 0
  const percent = total > 0 ? Math.min(100, Math.round(downloaded / total * 100)) : 0
  const stateLabel = progress?.state === 'verifying'
    ? t.replaceTranslatePackVerifying
    : progress?.state === 'extracting'
      ? t.replaceTranslatePackExtracting
      : progress?.state === 'retrying'
        ? t.replaceTranslatePackRetrying
        : t.rapidOcrDownloading

  return (
    <div className="kv-panel mt-0 w-full space-y-3">
      {/* 换档后的状态刷新由 SettingsShell 里 keyed 于 rapidOcrTier 的 useEffect 负责，这里不再手动 onRefresh。 */}
      <TierSelect tier={tier} t={t} onChangeTier={onChangeTier} />

      <div className="flex items-start gap-2">
        <span className={`mt-1 inline-block w-1.5 h-1.5 rounded-full ${status?.ready ? 'bg-emerald-500' : 'bg-amber-500'}`} />
        <div className="min-w-0 flex-1">
          <div className="kv-panel-title !mb-0">
            {status?.ready ? t.replaceTranslatePackReady : t.replaceTranslatePackMissing}
          </div>
          <div className="kv-panel-body">
            {status
              ? `${formatBytes(status.readyBytes)} / ${formatBytes(status.totalBytes)} · ${formatBytes(status.missingBytes)} missing`
              : '—'}
          </div>
          {status?.modelDir && <div className="kv-panel-body font-mono break-all">{status.modelDir}</div>}
        </div>
        <IconButton
          size="xs"
          onClick={() => onRefresh(tier)}
          disabled={downloadState === 'downloading'}
          label={t.rapidOcrRefresh}
        >
          <RefreshCw size={12} strokeWidth={2.25} />
        </IconButton>
      </div>

      {downloadState === 'downloading' && (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between gap-3 kv-panel-body">
            <span className="truncate">{stateLabel}: {progress?.fileName || ''}</span>
            <span className="tabular-nums shrink-0">{percent}%</span>
          </div>
          <div className="h-1.5 rounded-full overflow-hidden bg-neutral-900/10">
            <div className="h-full bg-blue-500 transition-[width]" style={{ width: `${percent}%` }} />
          </div>
          <div className="kv-panel-body flex justify-between gap-3">
            <span>{t.replaceTranslatePackProgress}</span>
            <span className="tabular-nums">{formatBytes(downloaded)} / {formatBytes(total)}</span>
          </div>
          {(progress?.attempt ?? 1) > 1 && (
            <div className="kv-panel-body">{t.replaceTranslatePackRetrying} · {progress?.attempt}/3</div>
          )}
        </div>
      )}

      {status?.files?.length ? (
        <div className="space-y-1 border-t border-neutral-900/[0.06] pt-2">
          {status.files.map(file => (
            <div key={`${file.componentId}:${file.fileName}`} className="flex items-center gap-2 kv-panel-body">
              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${file.ready ? 'bg-emerald-500' : file.state === 'invalid' ? 'bg-red-500' : 'bg-amber-500'}`} />
              <span className="font-mono truncate flex-1">{file.fileName}</span>
              <span className="tabular-nums shrink-0">{formatBytes(file.installedBytes)}</span>
            </div>
          ))}
        </div>
      ) : null}

      {!status?.ready && downloadState !== 'downloading' && (
        <Button variant="primary" onClick={() => onDownload(tier)}>
          <Download size={12} strokeWidth={2.5} />
          {downloadState === 'failed' ? t.replaceTranslatePackRetry : t.replaceTranslatePackDownload}
          {status && ` (${formatBytes(status.missingBytes)})`}
        </Button>
      )}
      {downloadState === 'failed' && downloadError && (
        <div className="kv-inline-error break-words">{t.rapidOcrDownloadFailed}: {downloadError}</div>
      )}
      <div className="kv-panel-body">{t.replaceTranslatePackHint}</div>
    </div>
  )
}

/**
 * 自定义提示词字段：设置页只显示状态与操作，正文在弹窗中编辑。
 * 空值表示使用内置默认（弹窗以默认模板作为编辑起点）；未改动直接保存不写入，
 * 保持空串以继续跟随内置默认。"恢复默认" 写回空串。
 */
export function PromptField({
  label,
  description,
  value,
  defaultText,
  t,
  onChange,
}: {
  label: string
  description?: string
  value: string
  defaultText: string
  t: I18n
  onChange: (value: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const customized = value !== ''

  return (
    <div className="kv-row">
      <div className="kv-row-text">
        <span className="kv-row-label">{label}</span>
        {description && <p className="kv-row-desc">{description}</p>}
      </div>
      <div className="kv-row-control flex items-center gap-2">
        <span className="text-[12px] text-neutral-500">
          {customized ? t.promptCustomized : defaultText ? t.promptUsingDefault : t.promptNotSet}
        </span>
        <Button
          size="sm"
          onClick={() => onChange('')}
          disabled={!defaultText || !customized}
          data-tauri-drag-region="false"
        >
          <RefreshCw size={10} />
          {t.restoreDefaultPrompt}
        </Button>
        <Button
          size="sm"
          onClick={() => setEditing(true)}
          aria-label={`${t.promptEdit} ${label}`}
          data-tauri-drag-region="false"
        >
          <Pencil size={10} />
          {t.promptEdit}
        </Button>
      </div>
      {editing && (
        <PromptEditorModal
          label={label}
          description={description}
          initial={value || defaultText}
          defaultText={defaultText}
          t={t}
          onSave={(next) => {
            if (next !== (value || defaultText)) onChange(next === defaultText ? '' : next)
            setEditing(false)
          }}
          onClose={() => setEditing(false)}
        />
      )}
    </div>
  )
}

function PromptEditorModal({
  label,
  description,
  initial,
  defaultText,
  t,
  onSave,
  onClose,
}: {
  label: string
  description?: string
  initial: string
  defaultText: string
  t: I18n
  onSave: (value: string) => void
  onClose: () => void
}) {
  const [draft, setDraft] = useState(initial)
  const modalRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const previous = document.activeElement
    modalRef.current?.querySelector('textarea')?.focus()
    return () => { if (previous instanceof HTMLElement) previous.focus() }
  }, [])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
      if (e.key === 'Tab') {
        const controls = modalRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), textarea')
        const first = controls?.[0]
        const last = controls?.[controls.length - 1]
        if (first && last && ((e.shiftKey && document.activeElement === first) || (!e.shiftKey && document.activeElement === last))) {
          e.preventDefault()
          const target = e.shiftKey ? last : first
          target.focus()
        }
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return createPortal(
    <div
      className="kv-modal-backdrop kv-modal-backdrop--portal"
      data-tauri-drag-region="false"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        className="kv kv-modal kv-prompt-editor outline-none"
        ref={modalRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        data-own-escape
        tabIndex={-1}
        data-tauri-drag-region="false"
      >
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold">{label}</h3>
            {description && <p className="kv-row-desc">{description}</p>}
          </div>
          <IconButton size="xs" label={t.cancel} onClick={onClose} data-tauri-drag-region="false">
            <X size={14} />
          </IconButton>
        </div>
        <TextArea
          value={draft}
          onChange={setDraft}
          rows={14}
          aria-label={label}
        />
        <div className="flex items-center justify-between gap-2">
          <Button
            size="sm"
            onClick={() => setDraft(defaultText)}
            disabled={!defaultText || draft === defaultText}
            data-tauri-drag-region="false"
          >
            <RefreshCw size={10} />
            {t.restoreDefaultPrompt}
          </Button>
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={onClose} data-tauri-drag-region="false">{t.cancel}</Button>
            <Button variant="primary" size="sm" onClick={() => onSave(draft)} data-tauri-drag-region="false">
              {t.save}
            </Button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  )
}

function RapidOcrStatusPanel({
  status,
  tier,
  downloadState,
  downloadError,
  t,
  onRefresh,
  onDownload,
  onChangeTier,
}: {
  status: RapidOcrStatus | null
  tier: RapidOcrTier
  downloadState: RapidOcrDownloadState
  downloadError: string
  t: I18n
  onRefresh: () => void
  onDownload: (tier: RapidOcrTier) => void
  onChangeTier: (tier: RapidOcrTier) => void
}) {
  const available = tier === 'high' ? status?.highAvailable : status?.standardAvailable
  return (
    <div className="kv-panel mt-0 w-full space-y-2.5">
      <TierSelect tier={tier} t={t} onChangeTier={onChangeTier} />
      <p className="kv-row-desc">{t.rapidOcrTierHint}</p>

      {available ? (
        <div className="flex items-start gap-2">
          <span className="mt-0.5 inline-block w-1.5 h-1.5 rounded-full bg-emerald-500" />
          <div className="flex-1">
            <div className="kv-panel-title !mb-0">
              {t.rapidOcrModelsFound}
            </div>
            {status?.modelDir && (
              <div className="kv-panel-body font-mono break-all">
                {status.modelDir}
              </div>
            )}
          </div>
          <IconButton
            size="xs"
            onClick={onRefresh}
            label={t.rapidOcrRefresh}
          >
            <RefreshCw size={12} strokeWidth={2.25} />
          </IconButton>
        </div>
      ) : (
        <div className="space-y-2.5">
          <div className="flex items-start gap-2">
            <span className="mt-0.5 inline-block w-1.5 h-1.5 rounded-full bg-amber-500" />
            <div className="flex-1 kv-panel-title !mb-0">
              {t.rapidOcrModelsNotFound}
            </div>
            <IconButton
              size="xs"
              className="disabled:opacity-40"
              onClick={onRefresh}
              disabled={downloadState === 'downloading'}
              label={t.rapidOcrRefresh}
            >
              <RefreshCw size={12} strokeWidth={2.25} />
            </IconButton>
          </div>

          {downloadState === 'downloading' ? (
            <div className="pl-3.5 flex items-center gap-2 kv-panel-body">
              <RefreshCw size={12} strokeWidth={2.25} className="animate-spin" />
              <span>{t.rapidOcrDownloading}</span>
            </div>
          ) : (
            <div className="pl-3.5">
              <Button
                variant="primary"
                onClick={() => onDownload(tier)}
              >
                <Download size={12} strokeWidth={2.5} />
                {t.rapidOcrDownloadButton}
              </Button>
            </div>
          )}

          {downloadState === 'failed' && downloadError && (
            <div className="kv-inline-error pl-3.5 break-words">
              {t.rapidOcrDownloadFailed}: {downloadError}
            </div>
          )}

          <div className="kv-panel-body pl-3.5">
            {t.rapidOcrHint}
          </div>
        </div>
      )}
    </div>
  )
}
