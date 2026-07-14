import { Download, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { type DefaultPromptTemplates, type OfflineModelProgress, type RapidOcrStatus, type ReplaceTranslationPackStatus, type Settings } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { formatBytes } from '../utils/formatBytes'
import { ModelPairSelect } from './ModelPairSelect'
import {
  Input,
  Select,
  SettingRow,
  SettingsGroup,
  TextArea,
  Toggle,
} from './components'
import { type I18n } from './i18n'

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
  onDownloadRapidOcr: () => void
  onRefreshReplacePack: () => void
  onDownloadReplacePack: () => void
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
  const ocrMode = screenshot?.ocrMode ?? 'cloud_vision'
  const cardWidth = screenshot?.cardWidth ?? 480
  const [widthDraft, setWidthDraft] = useState(String(cardWidth))
  // 边打字不 clamp（避免输 "5" 立刻跳 360）；失焦/回车时 clamp 到 360–720 再提交。
  const commitCardWidth = () => {
    const n = parseInt(widthDraft, 10)
    const next = Number.isFinite(n) ? Math.max(360, Math.min(720, n)) : cardWidth
    setWidthDraft(String(next))
    if (next !== cardWidth) onUpdate({ cardWidth: next })
  }

  return (
    <>
      <SettingsGroup title={t.sectionOptions}>
          <SettingRow label={t.enabled}>
            <Toggle
              checked={screenshot?.enabled ?? true}
              onChange={(enabled) => onUpdate({ enabled })}
            />
          </SettingRow>

          {screenshot?.enabled !== false && (
            <>
              <SettingRow
                label={t.screenshotShowOriginal}
                description={t.screenshotShowOriginalHint}
              >
                <Toggle
                  checked={!(screenshot?.directTranslate ?? false)}
                  onChange={(showOriginal) => onUpdate({ directTranslate: !showOriginal })}
                />
              </SettingRow>

              <SettingRow
                label={t.screenshotTranslationThinking}
                description={t.screenshotTranslationThinkingHint}
              >
                <Toggle
                  checked={screenshot?.thinkingEnabled ?? false}
                  onChange={(thinkingEnabled) => onUpdate({ thinkingEnabled })}
                />
              </SettingRow>

              <SettingRow label={t.screenshotTranslationStream}>
                <Toggle
                  checked={screenshot?.streamEnabled !== false}
                  onChange={(streamEnabled) => onUpdate({ streamEnabled })}
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
                  className="w-24"
                  value={widthDraft}
                  onChange={setWidthDraft}
                  onBlur={commitCardWidth}
                  onKeyDown={(e) => { if (e.key === 'Enter') commitCardWidth() }}
                />
              </SettingRow>
            </>
          )}
      </SettingsGroup>

      {screenshot?.enabled !== false && (
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
                downloadState={replacePackDownloadState}
                downloadError={replacePackDownloadError}
                progress={replacePackProgress}
                t={t}
                onRefresh={onRefreshReplacePack}
                onDownload={onDownloadReplacePack}
              />
            </SettingRow>
          )}
        </SettingsGroup>
      )}

      {screenshot?.enabled !== false && (
        <>
          {hasSystemOcr && (
            <SettingsGroup title={t.ocrEngine}>
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
                      className="w-44"
                    />
                  </SettingRow>

                  {ocrMode === 'system' && (
                    <div className="kv-panel mt-2">
                      <div className="kv-panel-body">
                      {isMac ? t.ocrEngineMacHint : t.ocrEngineWindowsHint}
                      </div>
                    </div>
                  )}

                  {ocrMode === 'rapid_ocr' && (
                    <>
                      <p className="kv-row-desc px-1 pb-1">{t.ocrEngineRapidOcrSharedNote}</p>
                      <RapidOcrStatusPanel
                        status={rapidOcrStatus}
                        downloadState={rapidOcrDownloadState}
                        downloadError={rapidOcrDownloadError}
                        t={t}
                        onRefresh={onRefreshRapidOcrStatus}
                        onDownload={onDownloadRapidOcr}
                      />
                    </>
                  )}
            </SettingsGroup>
          )}

          <SettingsGroup title={t.sectionModel}>
              <SettingRow label={t.selectModelPair}>
                <ModelPairSelect
                  providerId={screenshot.providerId}
                  model={screenshot.model}
                  providers={settings.providers}
                  onChange={(providerId, model) => onUpdate({ providerId, model })}
                />
              </SettingRow>
          </SettingsGroup>

          <SettingsGroup title={t.sectionPrompt}>
              <PromptField
                label={t.screenshotTranslationPrompt}
                description={t.screenshotTranslationPromptHint}
                value={screenshot?.prompt || ''}
                defaultText={defaultPrompts?.screenshotTranslationTemplate || ''}
                restoreLabel={t.restoreDefaultPrompt}
                onChange={(prompt) => onUpdate({ prompt })}
              />
              <PromptField
                label={t.selectedTextTranslationPrompt}
                description={t.selectedTextTranslationPromptHint}
                value={screenshot?.textPrompt || ''}
                defaultText={defaultPrompts?.selectedTextTranslationTemplate || ''}
                restoreLabel={t.restoreDefaultPrompt}
                onChange={(textPrompt) => onUpdate({ textPrompt })}
              />
              <PromptField
                label={t.replaceTranslationPrompt}
                description={t.replaceTranslationPromptHint}
                value={screenshot?.replacePrompt || ''}
                defaultText={defaultPrompts?.replaceTranslationTemplate || ''}
                restoreLabel={t.restoreDefaultPrompt}
                onChange={(replacePrompt) => onUpdate({ replacePrompt })}
              />
          </SettingsGroup>
        </>
      )}
    </>
  )
}

function ReplaceTranslationPackPanel({
  status,
  downloadState,
  downloadError,
  progress,
  t,
  onRefresh,
  onDownload,
}: {
  status: ReplaceTranslationPackStatus | null
  downloadState: ReplacePackDownloadState
  downloadError: string
  progress: OfflineModelProgress | null
  t: I18n
  onRefresh: () => void
  onDownload: () => void
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
          onClick={onRefresh}
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
          <div className="h-1.5 rounded-full overflow-hidden bg-black/10 dark:bg-white/10">
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
        <div className="space-y-1 border-t border-black/[0.06] dark:border-white/[0.07] pt-2">
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
        <Button variant="primary" onClick={onDownload}>
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
 * 自定义提示词字段：空值时把默认模板预填进文本框（可编辑起点），
 * 用户未编辑前保存仍写空串（运行时用内置默认）；"恢复默认" 清空并复位预填。
 */
export function PromptField({
  label,
  description,
  value,
  defaultText,
  restoreLabel,
  onChange,
}: {
  label: string
  description?: string
  value: string
  defaultText: string
  restoreLabel: string
  onChange: (value: string) => void
}) {
  const [interacted, setInteracted] = useState(false)
  const shown = interacted ? value : value || defaultText

  return (
    <div className="py-2">
      <div className="mb-2 flex items-start justify-between gap-2">
        <div>
          <div className="kv-row-label">{label}</div>
          {description && <p className="kv-row-desc">{description}</p>}
        </div>
        <Button
          size="sm"
          className="shrink-0"
          onClick={() => {
            setInteracted(false)
            onChange('')
          }}
          disabled={!defaultText || (!value && !interacted)}
          data-tauri-drag-region="false"
        >
          <RefreshCw size={10} />
          {restoreLabel}
        </Button>
      </div>
      <TextArea
        value={shown}
        onChange={(v) => {
          setInteracted(true)
          onChange(v)
        }}
        rows={4}
      />
    </div>
  )
}

function RapidOcrStatusPanel({
  status,
  downloadState,
  downloadError,
  t,
  onRefresh,
  onDownload,
}: {
  status: RapidOcrStatus | null
  downloadState: RapidOcrDownloadState
  downloadError: string
  t: I18n
  onRefresh: () => void
  onDownload: () => void
}) {
  const available = status?.available
  return (
    <div className="kv-panel mt-0 w-full space-y-2.5">
      <p className="kv-row-desc">{t.rapidOcrHint2}</p>

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
                onClick={onDownload}
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
