import type { CSSProperties, MouseEvent } from 'react'
import { flushSync } from 'react-dom'
import { Check, Monitor, Moon, Sun } from 'lucide-react'
import type { Settings } from '../../api/tauri'
import type { I18n, Lang } from '../../components/i18n'
import { themeDisplayName } from '../../settings/public/themes'
import { BUILTIN_THEMES } from '../../theme/theme'
import type { ThemeDefinition, ThemeMode } from '../../theme/types'
import { useDocumentDark } from '../../theme/useDocumentDark'
import { OnboardingBlock, OnboardingStepFrame } from '../OnboardingStepFrame'

type AppearanceStepProps = {
  t: I18n
  lang: Lang
  settings: Settings
  onChange: (settings: Settings) => void
}

type ViewTransitionDocument = Document & {
  startViewTransition?: (update: () => void) => { ready: Promise<void>; finished: Promise<void> }
}

/**
 * 以点击处为圆心揭开新外观。只负责过渡；真正的主题应用由 OnboardingShell 的预览 effect 完成，
 * 它在同步提交里执行，因此 View Transition 截到的就是新画面。不支持或减少动态效果时直接切换。
 */
function revealFrom(event: MouseEvent, commit: () => void) {
  const doc = document as ViewTransitionDocument
  const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
  if (!doc.startViewTransition || reduced) {
    commit()
    return
  }
  const x = event.clientX || window.innerWidth / 2
  const y = event.clientY || window.innerHeight / 2
  const radius = Math.hypot(Math.max(x, window.innerWidth - x), Math.max(y, window.innerHeight - y))
  const root = document.documentElement
  root.dataset.obReveal = 'true'
  const transition = doc.startViewTransition(() => flushSync(commit))
  transition.ready.then(() => {
    root.animate(
      { clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${radius}px at ${x}px ${y}px)`] },
      { duration: 680, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', pseudoElement: '::view-transition-new(root)' },
    )
  }).catch(() => {})
  transition.finished.catch(() => {}).finally(() => { delete root.dataset.obReveal })
}

function swatchStyle(theme: ThemeDefinition): CSSProperties {
  return {
    '--sw-ls': theme.light.surface,
    '--sw-lm': theme.light.surfaceMuted,
    '--sw-lt': theme.light.text,
    '--sw-la': theme.light.accent,
    '--sw-ds': theme.dark.surface,
    '--sw-dm': theme.dark.surfaceMuted,
    '--sw-dt': theme.dark.text,
    '--sw-da': theme.dark.accent,
  } as CSSProperties
}

/** 外观插画：一扇用当前主题语义色绘制的迷你 Kivio 窗口，外加随明暗换位的日月。 */
function MiniWindowArt({ t }: { t: I18n }) {
  const dark = useDocumentDark()
  return (
    <div className="ob-mini">
      <span className={`ob-mini-orb${dark ? ' is-dark' : ''}`}>
        <Sun size={18} className="ob-mini-sun" />
        <Moon size={16} className="ob-mini-moon" />
      </span>
      <div className="ob-mini-window">
        <div className="ob-mini-side">
          <span className="ob-mini-dots"><i /><i /><i /></span>
          <span className="ob-mini-row is-active" />
          <span className="ob-mini-row" />
          <span className="ob-mini-row ob-mini-row--short" />
          <span className="ob-mini-row" />
        </div>
        <div className="ob-mini-main">
          <span className="ob-mini-hello">{t.onboardingAppearanceDemoGreeting}</span>
          <span className="ob-mini-user">{t.onboardingAppearanceDemoPrompt}</span>
          <span className="ob-mini-lines"><i /><i /><i /></span>
          <span className="ob-mini-composer">
            <span>{t.onboardingAppearanceDemoComposer}</span>
            <b />
          </span>
        </div>
      </div>
    </div>
  )
}

export function AppearanceStep({ t, lang, settings, onChange }: AppearanceStepProps) {
  const themes = [...BUILTIN_THEMES, ...(settings.customThemes ?? [])]
  const modes: { value: ThemeMode; label: string; icon: typeof Sun }[] = [
    { value: 'system', label: t.themeSystem, icon: Monitor },
    { value: 'light', label: t.themeLight, icon: Sun },
    { value: 'dark', label: t.themeDark, icon: Moon },
  ]
  const langs: { value: Lang; glyph: string; label: string }[] = [
    { value: 'zh', glyph: '文', label: '简体中文' },
    { value: 'en', glyph: 'Aa', label: 'English' },
  ]
  const currentLang = (settings.settingsLanguage || 'zh') as Lang

  return (
    <OnboardingStepFrame
      index={1}
      eyebrow={t.onboardingStepAppearance}
      title={t.onboardingAppearanceTitle}
      subtitle={t.onboardingAppearanceDesc}
      art={<MiniWindowArt t={t} />}
    >
      <OnboardingBlock order={0} label={t.language}>
        <div className="ob-choice-row" role="radiogroup" aria-label={t.language}>
          {langs.map((item) => (
            <button
              key={item.value}
              type="button"
              role="radio"
              aria-checked={currentLang === item.value}
              className={`ob-choice${currentLang === item.value ? ' is-selected' : ''}`}
              onClick={() => onChange({ ...settings, settingsLanguage: item.value })}
              data-tauri-drag-region="false"
            >
              <span className="ob-choice-glyph">{item.glyph}</span>
              <span className="ob-choice-label">{item.label}</span>
            </button>
          ))}
        </div>
      </OnboardingBlock>

      <OnboardingBlock order={1} label={t.onboardingAppearanceMode}>
        <div className="ob-choice-row ob-choice-row--three" role="radiogroup" aria-label={t.onboardingAppearanceMode}>
          {modes.map(({ value, label, icon: Icon }) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={settings.theme === value}
              className={`ob-choice${settings.theme === value ? ' is-selected' : ''}`}
              onClick={(event) => {
                if (settings.theme === value) return
                revealFrom(event, () => onChange({ ...settings, theme: value }))
              }}
              data-tauri-drag-region="false"
            >
              <span className="ob-choice-glyph"><Icon size={16} /></span>
              <span className="ob-choice-label">{label}</span>
            </button>
          ))}
        </div>
      </OnboardingBlock>

      <OnboardingBlock
        order={2}
        label={t.onboardingAppearancePalette}
        aside={t.onboardingAppearancePaletteHint}
      >
        <div className="ob-swatches" role="radiogroup" aria-label={t.onboardingAppearancePalette}>
          {themes.map((theme, index) => {
            const builtin = BUILTIN_THEMES.some((item) => item.id === theme.id)
            const name = themeDisplayName(theme, lang, builtin)
            const selected = settings.themeColor === theme.id
            return (
              <button
                key={theme.id}
                type="button"
                role="radio"
                aria-checked={selected}
                aria-label={name}
                className={`ob-swatch${selected ? ' is-selected' : ''}`}
                style={{ ...swatchStyle(theme), '--k': index } as CSSProperties}
                onClick={(event) => {
                  if (selected) return
                  revealFrom(event, () => onChange({ ...settings, themeColor: theme.id }))
                }}
                data-tauri-drag-region="false"
              >
                <span className="ob-swatch-art">
                  <span className="ob-swatch-half ob-swatch-half--light"><i /><b /></span>
                  <span className="ob-swatch-half ob-swatch-half--dark"><i /><b /></span>
                  {selected ? <span className="ob-swatch-check"><Check size={11} strokeWidth={3} /></span> : null}
                </span>
                <span className="ob-swatch-name">{name}</span>
              </button>
            )
          })}
        </div>
      </OnboardingBlock>
    </OnboardingStepFrame>
  )
}
