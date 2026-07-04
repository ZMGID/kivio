import { Download, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { type DefaultPromptTemplates, type RapidOcrStatus, type Settings } from '../api/tauri'
import { ModelPairSelect } from './ModelPairSelect'
import {
  HotkeyInput,
  Input,
  Select,
  SettingRow,
  SettingsGroup,
  TextArea,
  Toggle,
} from './components'
import { type I18n } from './i18n'

type ScreenshotTranslation = Settings['screenshotTranslation']
type RecordingTarget = 'main' | 'screenshotTranslation' | 'screenshotTranslationText' | 'screenshotTranslationReplace' | 'lens'
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
  onToggleRecording: (target: 'screenshotTranslation' | 'screenshotTranslationText' | 'screenshotTranslationReplace') => void
  onRefreshRapidOcrStatus: () => void
  onDownloadRapidOcr: () => void
  hotkeyError?: string
  textHotkeyError?: string
  replaceHotkeyError?: string
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
  replaceHotkeyError,
  hotkeyClearLabel,
}: ScreenshotTranslationSettingsProps) {
  const screenshot = settings.screenshotTranslation
  const ocrMode = screenshot?.ocrMode ?? 'cloud_vision'
  const cardWidth = screenshot?.cardWidth ?? 480
  const [widthDraft, setWidthDraft] = useState(String(cardWidth))
  // 邊打字不 clamp（避免輸 "5" 立刻跳 360）；失焦/回車時 clamp 到 360–720 再提交。
  const commitCardWidth = () => {
    const n = parseInt(widthDraft, 10)
    const next = Number.isFinite(n) ? Math.max(360, Math.min(720, n)) : cardWidth
    setWidthDraft(String(next))
    if (next !== cardWidth) onUpdate({ cardWidth: next })
  }

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
              <SettingRow label={t.screenshotHotkey} stack>
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

              <SettingRow label={t.screenshotTextHotkey} stack>
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
        <SettingsGroup title={t.replaceTranslate}>
          <SettingRow label={t.replaceTranslateHotkey} stack>
            <HotkeyInput
              value={screenshot?.replaceHotkey ?? ''}
              placeholder="CommandOrControl+Shift+R"
              recording={recordingTarget === 'screenshotTranslationReplace'}
              onToggleRecording={() => onToggleRecording('screenshotTranslationReplace')}
              recordLabel={t.hotkeyRecord}
              recordingLabel={t.hotkeyRecording}
              recordingPlaceholder={t.hotkeyRecordingPlaceholder}
              onClear={() => onUpdate({ replaceHotkey: '' })}
              clearLabel={hotkeyClearLabel}
              error={replaceHotkeyError}
            />
          </SettingRow>

          <SettingRow label={t.replaceTranslateEnabled}>
            <Toggle
              checked={screenshot?.replaceEnabled !== false}
              onChange={(replaceEnabled) => onUpdate({ replaceEnabled })}
            />
          </SettingRow>

          {hasSystemOcr && screenshot?.replaceEnabled !== false && (
            <SettingRow label={t.replaceTranslateRapidOcr} stack>
              <RapidOcrStatusPanel
                status={rapidOcrStatus}
                downloadState={rapidOcrDownloadState}
                downloadError={rapidOcrDownloadError}
                t={t}
                onRefresh={onRefreshRapidOcrStatus}
                onDownload={onDownloadRapidOcr}
              />
            </SettingRow>
          )}
        </SettingsGroup>
      )}

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
          </SettingsGroup>
        </>
      )}
    </>
  )
}

/**
 * 自訂提示詞欄位：空值時把預設範本預填進文字框（可編輯起點），
 * 使用者未編輯前儲存仍寫空串（執行時用內建預設）；"恢復預設" 清空並復位預填。
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
        <button
          type="button"
          className="kv-btn sm shrink-0"
          onClick={() => {
            setInteracted(false)
            onChange('')
          }}
          disabled={!defaultText || (!value && !interacted)}
          data-tauri-drag-region="false"
        >
          <RefreshCw size={10} />
          {restoreLabel}
        </button>
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
  return (
    <div className="kv-panel mt-0 w-full">
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
