import { ChevronRight, Download, RefreshCw } from 'lucide-react'
import { type DefaultPromptTemplates, type RapidOcrStatus, type Settings } from '../api/tauri'
import { ModelPairSelect } from './ModelPairSelect'
import {
  DefaultPrompt,
  HotkeyInput,
  Label,
  Select,
  SettingRow,
  SettingsGroup,
  TextArea,
  Toggle,
} from './components'
import { type I18n } from './i18n'

type ScreenshotTranslation = Settings['screenshotTranslation']
type RecordingTarget = 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens'
type RapidOcrDownloadState = 'idle' | 'downloading' | 'failed'

interface ScreenshotTranslationSettingsProps {
  settings: Settings
  isMac: boolean
  hasSystemOcr: boolean
  recordingTarget: RecordingTarget | null
  defaultPrompts: DefaultPromptTemplates | null
  rapidOcrStatus: RapidOcrStatus | null
  rapidOcrDownloadState: RapidOcrDownloadState
  rapidOcrDownloadError: string
  t: I18n
  onUpdate: (updates: Partial<ScreenshotTranslation>) => void
  onToggleRecording: (target: 'screenshotTranslation' | 'screenshotTranslationText') => void
  onRefreshRapidOcrStatus: () => void
  onDownloadRapidOcr: () => void
  hotkeyError?: string
  textHotkeyError?: string
  hotkeyClearLabel?: string
}

export function ScreenshotTranslationSettings({
  settings,
  isMac,
  hasSystemOcr,
  recordingTarget,
  defaultPrompts,
  rapidOcrStatus,
  rapidOcrDownloadState,
  rapidOcrDownloadError,
  t,
  onUpdate,
  onToggleRecording,
  onRefreshRapidOcrStatus,
  onDownloadRapidOcr,
  hotkeyError,
  textHotkeyError,
  hotkeyClearLabel,
}: ScreenshotTranslationSettingsProps) {
  const screenshot = settings.screenshotTranslation
  const ocrMode = screenshot?.ocrMode ?? 'cloud_vision'

  return (
    <>
      <SettingsGroup title={t.screenshotTranslate}>
          <SettingRow label={t.enabled}>
            <Toggle
              checked={screenshot?.enabled ?? true}
              onChange={(enabled) => onUpdate({ enabled })}
            />
          </SettingRow>

          {screenshot?.enabled !== false && (
            <>
              <SettingRow label={t.screenshotHotkey} description={t.screenshotHotkey} stack>
                <HotkeyInput
                  value={screenshot?.hotkey ?? ''}
                  placeholder="CommandOrControl+Shift+A"
                  recording={recordingTarget === 'screenshotTranslation'}
                  onToggleRecording={() => onToggleRecording('screenshotTranslation')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                  onClear={() => onUpdate({ hotkey: '' })}
                  clearLabel={hotkeyClearLabel}
                  error={hotkeyError}
                />
              </SettingRow>

              <SettingRow label={t.screenshotTextHotkey} description={t.selectedText} stack>
                <HotkeyInput
                  value={screenshot?.textHotkey ?? ''}
                  placeholder="CommandOrControl+Shift+T"
                  recording={recordingTarget === 'screenshotTranslationText'}
                  onToggleRecording={() => onToggleRecording('screenshotTranslationText')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                  onClear={() => onUpdate({ textHotkey: '' })}
                  clearLabel={hotkeyClearLabel}
                  error={textHotkeyError}
                />
              </SettingRow>
            </>
          )}
      </SettingsGroup>

      {screenshot?.enabled !== false && (
        <>
          <SettingsGroup title={t.screenshotTranslate}>
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

              <SettingRow
                label={t.screenshotTranslationStream}
                description={t.screenshotTranslationStreamHint}
              >
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
          </SettingsGroup>

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
                    <RapidOcrStatusPanel
                      status={rapidOcrStatus}
                      downloadState={rapidOcrDownloadState}
                      downloadError={rapidOcrDownloadError}
                      t={t}
                      onRefresh={onRefreshRapidOcrStatus}
                      onDownload={onDownloadRapidOcr}
                    />
                  )}
            </SettingsGroup>
          )}

          <SettingsGroup title={t.engine}>
              <SettingRow label={t.selectModelPair}>
                <ModelPairSelect
                  providerId={screenshot.providerId}
                  model={screenshot.model}
                  providers={settings.providers}
                  onChange={(providerId, model) => onUpdate({ providerId, model })}
                />
              </SettingRow>
          </SettingsGroup>

          <SettingsGroup title={t.customPrompts}>
              <details className="group">
                <summary className="kv-row cursor-pointer list-none">
                  <div className="kv-row-text">
                    <div className="kv-row-label flex items-center gap-1.5">
                      <ChevronRight size={13} className="text-neutral-400 dark:text-neutral-500 group-open:rotate-90 transition-transform duration-200" strokeWidth={2.25} />
                      {t.customPrompts}
                    </div>
                    <div className="kv-row-desc">{t.screenshotTranslationPromptHint}</div>
                  </div>
                </summary>
                <div className="pb-2 space-y-2">
                  <Label>{t.screenshotTranslationPrompt}</Label>
                  <TextArea
                    value={screenshot?.prompt || ''}
                    onChange={(prompt) => onUpdate({ prompt })}
                    placeholder={t.screenshotTranslationPromptHint}
                    rows={3}
                  />
                  {!screenshot?.prompt?.trim() && (defaultPrompts?.screenshotTranslationTemplate || defaultPrompts?.translationTemplate) && (
                    <DefaultPrompt
                      label={t.defaultTemplate}
                      content={defaultPrompts?.screenshotTranslationTemplate || defaultPrompts?.translationTemplate || ''}
                    />
                  )}
                </div>
              </details>
          </SettingsGroup>
        </>
      )}
    </>
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
  return (
    <div className="kv-panel mt-2">
      {status?.modelsAvailable ? (
        <div className="flex items-start gap-2">
          <span className="mt-0.5 inline-block w-1.5 h-1.5 rounded-full bg-emerald-500" />
          <div className="flex-1">
            <div className="kv-panel-title !mb-0">
              {t.rapidOcrModelsFound}
            </div>
            {status.modelDir && (
              <div className="kv-panel-body font-mono break-all">
                {status.modelDir}
              </div>
            )}
          </div>
          <button
            onClick={onRefresh}
            className="kv-icon-btn"
            title={t.rapidOcrRefresh}
          >
            <RefreshCw size={12} strokeWidth={2.25} />
          </button>
        </div>
      ) : (
        <div className="space-y-2.5">
          <div className="flex items-start gap-2">
            <span className="mt-0.5 inline-block w-1.5 h-1.5 rounded-full bg-amber-500" />
            <div className="flex-1 kv-panel-title !mb-0">
              {t.rapidOcrModelsNotFound}
            </div>
            <button
              onClick={onRefresh}
              disabled={downloadState === 'downloading'}
              className="kv-icon-btn disabled:opacity-40"
              title={t.rapidOcrRefresh}
            >
              <RefreshCw size={12} strokeWidth={2.25} />
            </button>
          </div>

          {downloadState === 'downloading' ? (
            <div className="pl-3.5 flex items-center gap-2 kv-panel-body">
              <RefreshCw size={12} strokeWidth={2.25} className="animate-spin" />
              <span>{t.rapidOcrDownloading}</span>
            </div>
          ) : (
            <div className="pl-3.5">
              <button
                onClick={onDownload}
                className="kv-btn primary"
              >
                <Download size={12} strokeWidth={2.5} />
                {t.rapidOcrDownloadButton}
              </button>
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
