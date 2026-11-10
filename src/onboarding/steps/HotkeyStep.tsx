import { useEffect, useState, type CSSProperties } from 'react'
import type { ReactNode } from 'react'
import type { Settings } from '../../api/tauri'
import { HotkeyInput, Select, Toggle } from '../../settings/public/controls'
import type { I18n } from '../../components/i18n'
import { buildHotkey, formatHotkey, getPlatform } from '../../settings/public/hotkeys'
import { OnboardingFormRow } from '../OnboardingFormRow'
import { OnboardingBlock, OnboardingStepFrame } from '../OnboardingStepFrame'

type RecordingTarget = 'chat' | 'main' | 'screenshot' | 'selectedText' | 'replace' | 'lens'

type HotkeyStepProps = {
  t: I18n
  settings: Settings
  onChange: (settings: Settings) => void
}

type HotkeyField = {
  id: RecordingTarget
  label: string
  hint: string
  value: string
  placeholder: string
  onClear: () => void
}

type HotkeySection = {
  title: string
  lead?: ReactNode
  fields: HotkeyField[]
}

/**
 * 「快捷键」插画：把正在关注的那一组快捷键画成实体键帽，依次按下；录到新组合时键帽重新落位。
 * 录制中显示等待按键的空键帽。
 */
function KeycapArt({ t, label, hotkey, recording }: { t: I18n; label: string; hotkey: string; recording: boolean }) {
  const keys = hotkey.trim() ? formatHotkey(hotkey, getPlatform()) : []
  return (
    <div className={`ob-keys${recording ? ' is-recording' : ''}`}>
      <div key={recording ? 'recording' : hotkey} className="ob-keys-row">
        {recording || keys.length === 0 ? (
          <span className="ob-cap ob-cap--ghost">{recording ? t.hotkeyRecordingPlaceholder : t.onboardingDoneNotConfigured}</span>
        ) : keys.map((key, index) => (
          <span key={`${key}-${index}`} className={`ob-cap${key.length > 1 ? ' ob-cap--wide' : ''}`} style={{ '--k': index } as CSSProperties}>
            <span className="ob-cap-face">{key}</span>
          </span>
        ))}
      </div>
      <div className="ob-keys-caption">
        <span className="ob-keys-dot" />
        {label}
      </div>
    </div>
  )
}

export function HotkeyStep({ t, settings, onChange }: HotkeyStepProps) {
  const [recordingTarget, setRecordingTarget] = useState<RecordingTarget | null>(null)
  const [focusTarget, setFocusTarget] = useState<RecordingTarget>('chat')

  useEffect(() => {
    if (!recordingTarget) return
    const handleKeyDown = (event: KeyboardEvent) => {
      event.preventDefault()
      event.stopPropagation()
      const hotkey = buildHotkey(event)
      if (!hotkey) return

      switch (recordingTarget) {
        case 'chat':
          onChange({ ...settings, chatHotkey: hotkey })
          break
        case 'main':
          onChange({ ...settings, hotkey })
          break
        case 'screenshot':
          onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, hotkey },
          })
          break
        case 'selectedText':
          onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, textHotkey: hotkey },
          })
          break
        case 'replace':
          onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, replaceHotkey: hotkey },
          })
          break
        case 'lens':
          onChange({
            ...settings,
            lens: { ...settings.lens, hotkey },
          })
          break
      }
      setRecordingTarget(null)
    }
    window.addEventListener('keydown', handleKeyDown, true)
    return () => window.removeEventListener('keydown', handleKeyDown, true)
  }, [onChange, recordingTarget, settings])

  const sections: HotkeySection[] = [
    {
      title: t.onboardingHotkeySectionChat,
      fields: [{
        id: 'chat',
        label: t.chatHotkeyLabel,
        hint: t.onboardingHotkeyChatHint,
        value: settings.chatHotkey || '',
        placeholder: 'CommandOrControl+Shift+K',
        onClear: () => onChange({ ...settings, chatHotkey: '' }),
      }],
    },
    {
      title: t.onboardingHotkeySectionLens,
      fields: [{
        id: 'lens',
        label: t.onboardingHotkeyLens,
        hint: t.onboardingHotkeyLensHint,
        value: settings.lens.hotkey,
        placeholder: 'CommandOrControl+Shift+G',
        onClear: () => onChange({
          ...settings,
          lens: { ...settings.lens, hotkey: '' },
        }),
      }],
    },
    {
      title: t.onboardingHotkeySectionTranslator,
      fields: [{
        id: 'main',
        label: t.onboardingHotkeyTranslator,
        hint: t.onboardingHotkeyTranslatorHint,
        value: settings.hotkey || '',
        placeholder: 'CommandOrControl+Alt+T',
        onClear: () => onChange({ ...settings, hotkey: '' }),
      }],
    },
    {
      title: t.onboardingHotkeySectionQuickTranslate,
      lead: (
        <OnboardingFormRow label={t.ocrEngine} hint={t.ocrEngineHint} stack>
          <Select
            className="w-full max-w-[260px]"
            value={settings.screenshotTranslation?.ocrMode ?? 'cloud_vision'}
            onChange={(value) => onChange({
              ...settings,
              screenshotTranslation: {
                ...settings.screenshotTranslation,
                ocrMode: value as 'cloud_vision' | 'system' | 'rapid_ocr',
              },
            })}
            options={[
              { value: 'cloud_vision', label: t.ocrEngineCloudVision },
              { value: 'system', label: t.ocrEngineSystem },
              { value: 'rapid_ocr', label: t.ocrEngineRapidOcr },
            ]}
          />
        </OnboardingFormRow>
      ),
      fields: [
        {
          id: 'screenshot',
          label: t.onboardingHotkeyScreenshot,
          hint: t.onboardingHotkeyScreenshotHint,
          value: settings.screenshotTranslation.hotkey,
          placeholder: 'CommandOrControl+Shift+A',
          onClear: () => onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, hotkey: '' },
          }),
        },
        {
          id: 'selectedText',
          label: t.onboardingHotkeySelectedText,
          hint: t.onboardingHotkeySelectedTextHint,
          value: settings.screenshotTranslation.textHotkey,
          placeholder: 'CommandOrControl+Shift+T',
          onClear: () => onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, textHotkey: '' },
          }),
        },
        {
          id: 'replace',
          label: t.onboardingHotkeyReplace,
          hint: t.onboardingHotkeyReplaceHint,
          value: settings.screenshotTranslation?.replaceHotkey || '',
          placeholder: 'CommandOrControl+Shift+R',
          onClear: () => onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, replaceHotkey: '' },
          }),
        },
      ],
    },
  ]

  const replaceEnabled = settings.screenshotTranslation.replaceEnabled !== false
  const allFields = sections.flatMap((section) => section.fields)
  const focused = allFields.find((field) => field.id === (recordingTarget ?? focusTarget)) ?? allFields[0]

  return (
    <OnboardingStepFrame
      index={4}
      eyebrow={t.onboardingStepHotkey}
      title={t.onboardingHotkeyTitle}
      subtitle={t.onboardingHotkeyDesc}
      art={<KeycapArt t={t} label={focused.label} hotkey={focused.value} recording={recordingTarget !== null} />}
    >
      {sections.map((section, sectionIndex) => (
        <OnboardingBlock key={section.title} order={sectionIndex} label={section.title}>
          {section.lead}
          {section.fields.map((field) => (
            <div
              key={field.id}
              className={`ob-hotkey-row${focused.id === field.id ? ' is-focused' : ''}`}
              onPointerEnter={() => { if (!recordingTarget) setFocusTarget(field.id) }}
              onFocus={() => { if (!recordingTarget) setFocusTarget(field.id) }}
            >
              <OnboardingFormRow
                label={field.label}
                hint={field.hint}
                stack
                extra={field.id === 'replace' ? (
                  <label className="onboarding-inline-toggle">
                    <Toggle
                      checked={replaceEnabled}
                      onChange={(enabled) => onChange({
                        ...settings,
                        screenshotTranslation: {
                          ...settings.screenshotTranslation,
                          replaceEnabled: enabled,
                        },
                      })}
                    />
                    <span>{t.onboardingHotkeyReplaceEnabled}</span>
                  </label>
                ) : undefined}
              >
                <div className={`onboarding-hotkey-control${field.id === 'replace' && !replaceEnabled ? ' onboarding-hotkey-control--disabled' : ''}`}>
                  <HotkeyInput
                    value={field.value}
                    placeholder={field.placeholder}
                    recording={recordingTarget === field.id}
                    onToggleRecording={() => {
                      if (field.id === 'replace' && !replaceEnabled) return
                      setFocusTarget(field.id)
                      setRecordingTarget((current) => (current === field.id ? null : field.id))
                    }}
                    recordLabel={t.hotkeyRecord}
                    recordingLabel={t.hotkeyRecording}
                    recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                    onClear={field.onClear}
                    clearLabel={t.hotkeyClear}
                  />
                </div>
              </OnboardingFormRow>
            </div>
          ))}
        </OnboardingBlock>
      ))}
    </OnboardingStepFrame>
  )
}
