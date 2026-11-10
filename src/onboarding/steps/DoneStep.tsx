import type { CSSProperties, ReactNode } from 'react'
import { Crop, MessageSquare, Workflow } from 'lucide-react'
import type { Settings } from '../../api/tauri'
import type { I18n, Lang } from '../../components/i18n'
import { KivioBlob } from '../../chat/public/mascot'
import { formatHotkey, getPlatform } from '../../settings/public/hotkeys'
import { themeDisplayName } from '../../settings/public/themes'
import { BUILTIN_THEMES, resolveTheme } from '../../theme/theme'
import { OnboardingBlock, OnboardingStepFrame } from '../OnboardingStepFrame'
import { webSearchConfigured } from '../validation'

type DoneStepProps = {
  t: I18n
  lang: Lang
  settings: Settings
}

function resolveModelLabel(settings: Settings, providerId: string, model: string): string {
  const provider = settings.providers.find((item) => item.id === providerId)
  if (!provider) return '—'
  return `${provider.name} · ${model || '—'}`
}

function SummaryRow({ label, value, muted = false }: { label: string; value: ReactNode; muted?: boolean }) {
  return (
    <div className="onboarding-summary-row">
      <span className="onboarding-summary-label">{label}</span>
      <span className={`onboarding-summary-value${muted ? ' is-muted' : ''}`}>{value}</span>
    </div>
  )
}

function HotkeyValue({ hotkey, empty }: { hotkey: string; empty: string }) {
  const value = hotkey.trim()
  if (!value) return <>{empty}</>
  const keys = formatHotkey(value, getPlatform())
  if (keys.length === 0) return <>{value}</>
  return (
    <span className="ob-kbd-group">
      {keys.map((key, index) => <kbd key={`${key}-${index}`} className="ob-kbd">{key}</kbd>)}
    </span>
  )
}

/** 终章插画：墨墨完成时的轻跃，周围一圈圈墨晕向外散开。 */
function FinaleArt() {
  return (
    <div className="ob-finale">
      <span className="ob-finale-ring" style={{ '--r': 0 } as CSSProperties} />
      <span className="ob-finale-ring" style={{ '--r': 1 } as CSSProperties} />
      <span className="ob-finale-ring" style={{ '--r': 2 } as CSSProperties} />
      {[0, 1, 2, 3, 4, 5, 6, 7].map((index) => (
        <span key={index} className="ob-finale-drop" style={{ '--d': index } as CSSProperties} />
      ))}
      <span className="ob-finale-core"><KivioBlob size={88} mood="done" /></span>
    </div>
  )
}

export function DoneStep({ t, lang, settings }: DoneStepProps) {
  const theme = resolveTheme(settings)
  const builtin = BUILTIN_THEMES.some((item) => item.id === theme.id)
  const modeLabel = settings.theme === 'dark' ? t.themeDark : settings.theme === 'light' ? t.themeLight : t.themeSystem
  const notSet = t.onboardingDoneNotConfigured

  const modelRows = [
    { label: t.onboardingProviderChatModel, providerId: settings.defaultModels.chat.providerId, model: settings.defaultModels.chat.model },
    { label: t.onboardingDoneQuickTranslateModel, providerId: settings.screenshotTranslation.providerId, model: settings.screenshotTranslation.model },
    { label: t.onboardingDoneLensModel, providerId: settings.lens.providerId || '', model: settings.lens.model || '' },
  ]

  const hotkeyRows = [
    { label: t.chatHotkeyLabel, value: settings.chatHotkey || '' },
    { label: t.onboardingDoneHotkeyLens, value: settings.lens.hotkey },
    { label: t.onboardingDoneHotkeyTranslator, value: settings.hotkey },
    { label: t.onboardingDoneHotkeyScreenshot, value: settings.screenshotTranslation.hotkey },
    { label: t.onboardingDoneHotkeySelectedText, value: settings.screenshotTranslation.textHotkey },
    {
      label: t.onboardingDoneHotkeyReplace,
      value: settings.screenshotTranslation.replaceEnabled === false ? '' : settings.screenshotTranslation.replaceHotkey || '',
    },
  ]

  const searchReady = webSearchConfigured(settings)
  const chatSearch = searchReady && settings.chatTools.nativeTools?.webSearch !== false
  const lensSearch = searchReady && Boolean(settings.lens.webSearch?.enabled)

  const nextSteps = [
    { icon: MessageSquare, text: t.onboardingDoneNextChat },
    { icon: Crop, text: t.onboardingDoneNextScreen },
    { icon: Workflow, text: t.onboardingDoneNextTasks },
  ]

  return (
    <OnboardingStepFrame
      index={5}
      eyebrow={t.onboardingStepDone}
      title={t.onboardingDoneTitle}
      subtitle={t.onboardingDoneDesc}
      art={<FinaleArt />}
      className="ob-chapter--finale"
    >
      <OnboardingBlock order={0} label={t.onboardingDoneNextTitle}>
        <ol className="ob-next">
          {nextSteps.map(({ icon: Icon, text }, index) => (
            <li key={text} style={{ '--k': index } as CSSProperties}>
              <span className="ob-next-icon"><Icon size={15} /></span>
              <span>{text}</span>
            </li>
          ))}
        </ol>
      </OnboardingBlock>

      <div className="ob-receipts">
        <OnboardingBlock order={1} label={t.onboardingStepAppearance}>
          <div className="onboarding-summary-list">
            <SummaryRow label={t.language} value={(settings.settingsLanguage || 'zh') === 'en' ? 'English' : '简体中文'} />
            <SummaryRow label={t.onboardingAppearanceMode} value={modeLabel} />
            <SummaryRow
              label={t.onboardingAppearancePalette}
              value={(
                <span className="ob-theme-chip">
                  <i style={{ background: theme.light.accent }} />
                  {themeDisplayName(theme, lang, builtin)}
                </span>
              )}
            />
          </div>
        </OnboardingBlock>

        <OnboardingBlock order={2} label={t.onboardingDoneSectionModels}>
          <div className="onboarding-summary-list">
            {modelRows.map((row) => {
              const value = resolveModelLabel(settings, row.providerId, row.model)
              return <SummaryRow key={row.label} label={row.label} value={value} muted={value === '—'} />
            })}
            <SummaryRow
              label={`${t.webSearchChatSection} · ${t.onboardingDoneWebSearch}`}
              value={chatSearch ? t.enabled : t.onboardingDoneDisabled}
              muted={!chatSearch}
            />
            <SummaryRow
              label={`${t.webSearchLensSection} · ${t.onboardingDoneWebSearch}`}
              value={lensSearch ? t.enabled : t.onboardingDoneDisabled}
              muted={!lensSearch}
            />
          </div>
        </OnboardingBlock>

        <OnboardingBlock order={3} label={t.onboardingDoneHotkeys}>
          <div className="onboarding-summary-list">
            {hotkeyRows.map((row) => (
              <SummaryRow key={row.label} label={row.label} value={<HotkeyValue hotkey={row.value} empty={notSet} />} muted={!row.value.trim()} />
            ))}
          </div>
        </OnboardingBlock>
      </div>
    </OnboardingStepFrame>
  )
}
