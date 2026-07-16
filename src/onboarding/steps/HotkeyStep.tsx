import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import type { Settings } from '../../api/tauri'
import { HotkeyInput, Select, Toggle } from '../../settings/components'
import type { I18n } from '../../settings/i18n'
import { buildHotkey } from '../../settings/utils'
import { OnboardingFormRow } from '../OnboardingFormRow'
import { OnboardingStepFrame } from '../OnboardingStepFrame'

type RecordingTarget = 'main' | 'screenshot' | 'selectedText' | 'replace' | 'lens'

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

export function HotkeyStep({ t, settings, onChange }: HotkeyStepProps) {
  const [recordingTarget, setRecordingTarget] = useState<RecordingTarget | null>(null)

  useEffect(() => {
    if (!recordingTarget) return
    const handleKeyDown = (event: KeyboardEvent) => {
      event.preventDefault()
      event.stopPropagation()
      const hotkey = buildHotkey(event)
      if (!hotkey) return

      switch (recordingTarget) {
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
      title: t.onboardingHotkeySectionLens,
      fields: [{
        id: 'lens',
        label: t.onboardingHotkeyLens,
        hint: t.onboardingHotkeyLensHint,
        value: settings.lens?.hotkey || 'CommandOrControl+Shift+G',
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
        value: settings.hotkey || 'CommandOrControl+Alt+T',
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
          value: settings.screenshotTranslation?.hotkey || 'CommandOrControl+Shift+A',
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
          value: settings.screenshotTranslation?.textHotkey || 'CommandOrControl+Shift+T',
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
          value: settings.screenshotTranslation?.replaceHotkey || 'CommandOrControl+Shift+R',
          placeholder: 'CommandOrControl+Shift+R',
          onClear: () => onChange({
            ...settings,
            screenshotTranslation: { ...settings.screenshotTranslation, replaceHotkey: '' },
          }),
        },
      ],
    },
  ]

  const replaceEnabled = settings.screenshotTranslation?.replaceEnabled !== false

  return (
    <OnboardingStepFrame title={t.onboardingHotkeyTitle} subtitle={t.onboardingHotkeyDesc}>
      {sections.map((section) => (
        <div key={section.title} className="onboarding-section">
          <div className="onboarding-section-label">{section.title}</div>
          <div className="onboarding-card onboarding-card--rows">
            {section.lead}
            {section.fields.map((field) => (
              <OnboardingFormRow
                key={field.id}
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
            ))}
          </div>
        </div>
      ))}
    </OnboardingStepFrame>
  )
}
