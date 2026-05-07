import { Camera, ChevronRight, Download, RefreshCw } from 'lucide-react'
import { type DefaultPromptTemplates, type RapidOcrStatus, type Settings } from '../api/tauri'
import { ModelPairSelect } from './ModelPairSelect'
import {
  DefaultPrompt,
  HotkeyInput,
  Label,
  SectionTitle,
  Select,
  SettingRow,
  TextArea,
  Toggle,
} from './components'
import { type I18n } from './i18n'
import { type Platform } from './utils'

type ScreenshotTranslation = Settings['screenshotTranslation']
type RecordingTarget = 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'lens'
type RapidOcrDownloadState = 'idle' | 'downloading' | 'failed'

interface ScreenshotTranslationSettingsProps {
  settings: Settings
  platform: Platform
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
}

export function ScreenshotTranslationSettings({
  settings,
  platform,
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
}: ScreenshotTranslationSettingsProps) {
  const screenshot = settings.screenshotTranslation
  const ocrMode = screenshot?.ocrMode ?? 'cloud_vision'

  return (
    <section>
      <SectionTitle icon={Camera}>{t.screenshotTranslate}</SectionTitle>
      <div className="settings-card overflow-hidden">
        <div className="divide-y divide-black/[0.04] dark:divide-white/[0.05]">
          <SettingRow label={t.enabled}>
            <Toggle
              checked={screenshot?.enabled ?? true}
              onChange={(enabled) => onUpdate({ enabled })}
            />
          </SettingRow>

          {screenshot?.enabled !== false && (
            <>
              <div className="px-4 py-3 space-y-1.5">
                <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">
                  {t.screenshotHotkey}
                </span>
                <HotkeyInput
                  value={screenshot?.hotkey || 'CommandOrControl+Shift+A'}
                  placeholder="CommandOrControl+Shift+A"
                  recording={recordingTarget === 'screenshotTranslation'}
                  onToggleRecording={() => onToggleRecording('screenshotTranslation')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                />
              </div>

              <div className="px-4 py-3 space-y-1.5">
                <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-200">
                  {t.screenshotTextHotkey}
                </span>
                <HotkeyInput
                  value={screenshot?.textHotkey || 'CommandOrControl+Shift+T'}
                  placeholder="CommandOrControl+Shift+T"
                  recording={recordingTarget === 'screenshotTranslationText'}
                  onToggleRecording={() => onToggleRecording('screenshotTranslationText')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                />
              </div>

              <SettingRow label={t.selectModelPair}>
                <ModelPairSelect
                  providerId={screenshot.providerId}
                  model={screenshot.model}
                  providers={settings.providers}
                  platform={platform}
                  onChange={(providerId, model) => onUpdate({ providerId, model })}
                />
              </SettingRow>

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

              {hasSystemOcr && (
                <>
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
                    <div className="px-4 py-2 text-[11px] text-neutral-500 dark:text-neutral-400 border-t border-black/[0.04] dark:border-white/[0.05]">
                      {isMac ? t.ocrEngineMacHint : t.ocrEngineWindowsHint}
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
                </>
              )}

              <SettingRow label={t.lensKeepFullscreen} description={t.lensKeepFullscreenHint}>
                <Toggle
                  checked={screenshot?.keepFullscreenAfterCapture !== false}
                  onChange={(keepFullscreenAfterCapture) => onUpdate({ keepFullscreenAfterCapture })}
                />
              </SettingRow>

              <details className="group border-t border-black/[0.04] dark:border-white/[0.05]">
                <summary className="flex items-center gap-1.5 cursor-pointer text-[12px] font-medium text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-black/[0.02] dark:hover:bg-white/[0.025] transition-colors list-none px-4 py-3">
                  <ChevronRight size={13} className="text-neutral-400 dark:text-neutral-500 group-open:rotate-90 transition-transform duration-200" strokeWidth={2.25} />
                  {t.customPrompts}
                </summary>
                <div className="px-4 pb-4 space-y-2">
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
            </>
          )}
        </div>
      </div>
    </section>
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
    <div className="border-t border-black/[0.04] dark:border-white/[0.05] px-4 py-3 space-y-2 text-[12px]">
      {status?.modelsAvailable ? (
        <div className="flex items-start gap-2">
          <span className="mt-0.5 inline-block w-1.5 h-1.5 rounded-full bg-emerald-500" />
          <div className="flex-1">
            <div className="text-neutral-700 dark:text-neutral-200">
              {t.rapidOcrModelsFound}
            </div>
            {status.modelDir && (
              <div className="text-[11px] text-neutral-400 dark:text-neutral-500 mt-0.5 font-mono break-all">
                {status.modelDir}
              </div>
            )}
          </div>
          <button
            onClick={onRefresh}
            className="text-[11px] text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200"
            title={t.rapidOcrRefresh}
          >
            <RefreshCw size={12} strokeWidth={2.25} />
          </button>
        </div>
      ) : (
        <div className="space-y-2.5">
          <div className="flex items-start gap-2">
            <span className="mt-0.5 inline-block w-1.5 h-1.5 rounded-full bg-amber-500" />
            <div className="flex-1 text-neutral-700 dark:text-neutral-200">
              {t.rapidOcrModelsNotFound}
            </div>
            <button
              onClick={onRefresh}
              disabled={downloadState === 'downloading'}
              className="text-[11px] text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 disabled:opacity-40"
              title={t.rapidOcrRefresh}
            >
              <RefreshCw size={12} strokeWidth={2.25} />
            </button>
          </div>

          {downloadState === 'downloading' ? (
            <div className="pl-3.5 flex items-center gap-2 text-[11px] text-neutral-600 dark:text-neutral-300">
              <RefreshCw size={12} strokeWidth={2.25} className="animate-spin" />
              <span>{t.rapidOcrDownloading}</span>
            </div>
          ) : (
            <div className="pl-3.5">
              <button
                onClick={onDownload}
                className="text-[12px] px-3 py-1 rounded bg-blue-600 hover:bg-blue-700 text-white inline-flex items-center gap-1"
              >
                <Download size={12} strokeWidth={2.5} />
                {t.rapidOcrDownloadButton}
              </button>
            </div>
          )}

          {downloadState === 'failed' && downloadError && (
            <div className="pl-3.5 text-[11px] text-red-500 break-words">
              {t.rapidOcrDownloadFailed}: {downloadError}
            </div>
          )}

          <div className="text-[11px] text-neutral-500 dark:text-neutral-400 leading-5 pl-3.5">
            {t.rapidOcrHint}
          </div>
        </div>
      )}
    </div>
  )
}
