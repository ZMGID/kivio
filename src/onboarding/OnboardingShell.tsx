import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { ArrowLeft, ArrowRight, Check } from 'lucide-react'
import { type Settings, type SettingsVersion } from '../api/tauri'
import { getSettingsSnapshotCached, saveSettingsSnapshotCached, updateSettingsCached } from '../api/settingsCache'
import { i18n, type I18n, type Lang } from '../components/i18n'
import { usesNativeTitlebar } from '../utils/windowPlatform'
import { Button } from '../components/Button'
import { applyThemeSettings } from '../theme/theme'
import { ONBOARDING_STEPS, type OnboardingStepId } from './types'
import { canCompleteOnboarding, validateProviderStep } from './validation'
import { OnboardingAtmosphere } from './OnboardingArt'
import { AppearanceStep } from './steps/AppearanceStep'
import { DoneStep } from './steps/DoneStep'
import { HotkeyStep } from './steps/HotkeyStep'
import { ProviderStep } from './steps/ProviderStep'
import { WebSearchStep } from './steps/WebSearchStep'
import { WelcomeStep } from './steps/WelcomeStep'

type OnboardingShellProps = {
  onComplete: () => void
  onSkip: () => void
  onSettingsChange?: () => void
  /** 非 macOS 的窗口三键。引导页铺满整个窗口，由宿主传入，放在引导顶栏右端。 */
  windowControls?: ReactNode
}

type ThemeSelection = Pick<Settings, 'theme' | 'themeColor' | 'customThemes'>

/** 首次运行按系统语言（浏览器/系统 locale）自动选定界面语言：中文 locale → zh，其余 → en。 */
function detectSystemLang(): Lang {
  const raw = (
    (typeof navigator !== 'undefined' && (navigator.language || navigator.languages?.[0])) || ''
  ).toLowerCase()
  return raw.startsWith('zh') ? 'zh' : 'en'
}

function stepLabels(t: I18n): Record<OnboardingStepId, string> {
  return {
    welcome: t.onboardingStepWelcome,
    appearance: t.onboardingStepAppearance,
    provider: t.onboardingStepProvider,
    webSearch: t.onboardingStepWebSearch,
    hotkey: t.onboardingStepHotkey,
    done: t.onboardingStepDone,
  }
}

export function OnboardingShell({ onComplete, onSkip, onSettingsChange, windowControls }: OnboardingShellProps) {
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [settings, setSettings] = useState<Settings | null>(null)
  const [stepIndex, setStepIndex] = useState(0)
  const [direction, setDirection] = useState<'forward' | 'back'>('forward')
  const [skipConfirmOpen, setSkipConfirmOpen] = useState(false)
  const [providerBypass, setProviderBypass] = useState(false)
  const settingsVersionRef = useRef<SettingsVersion | null>(null)
  /** 已保存的主题选择。外观步只做预览；未保存就离开时恢复它。 */
  const savedThemeRef = useRef<ThemeSelection | null>(null)

  const stepId = ONBOARDING_STEPS[stepIndex] ?? 'welcome'
  const lang = (settings?.settingsLanguage || 'zh') as Lang
  const t = i18n[lang]

  const loadSettings = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    setSaveError(null)
    try {
      const snapshot = await getSettingsSnapshotCached()
      const loaded = snapshot.settings
      settingsVersionRef.current = snapshot.version
      savedThemeRef.current = { theme: loaded.theme, themeColor: loaded.themeColor, customThemes: loaded.customThemes }
      // 首次运行按系统语言自动设定界面语言（欢迎页起即本地化）；但若用户此前已选过语言
      // （如重跑引导的老用户），沿用其选择，不要用系统 locale 覆盖。
      setSettings({
        ...loaded,
        settingsLanguage: loaded.settingsLanguage || detectSystemLang(),
      })
    } catch (err) {
      console.error('Failed to load settings for onboarding:', err)
      setLoadError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void loadSettings()
  }, [loadSettings])

  // 主题预览：草稿里的明暗与主题色直接画到窗口上（同步于提交，供外观步的揭幕过渡截取新画面）。
  // 保存成功后由宿主按已保存设置重新应用；未保存就卸载时恢复原主题。
  const previewTheme = settings?.theme
  const previewThemeColor = settings?.themeColor
  const previewCustomThemes = settings?.customThemes
  useLayoutEffect(() => {
    if (!previewTheme || !previewThemeColor) return
    applyThemeSettings({ theme: previewTheme, themeColor: previewThemeColor, customThemes: previewCustomThemes })
  }, [previewCustomThemes, previewTheme, previewThemeColor])
  useEffect(() => () => {
    const saved = savedThemeRef.current
    if (saved) applyThemeSettings(saved)
  }, [])

  const updateSettings = useCallback((next: Settings) => {
    setSettings(next)
  }, [])

  const providerValidation = useMemo(
    () => (settings ? validateProviderStep(settings) : { ok: false }),
    [settings],
  )

  const canAdvanceFromProvider = providerValidation.ok || providerBypass
  const canGoNext = stepId !== 'provider' || canAdvanceFromProvider

  const persistSettings = useCallback(async (status: 'completed' | 'skipped') => {
    if (!settings) return false
    setSaving(true)
    setSaveError(null)
    try {
      const expectedVersion = settingsVersionRef.current
      if (!expectedVersion) throw new Error('Settings version is unavailable')
      const saved = await saveSettingsSnapshotCached({
        ...settings,
        onboardingStatus: status,
      }, expectedVersion)
      settingsVersionRef.current = saved.version
      savedThemeRef.current = {
        theme: saved.settings.theme,
        themeColor: saved.settings.themeColor,
        customThemes: saved.settings.customThemes,
      }
      setSettings(saved.settings)
      onSettingsChange?.()
      return true
    } catch (err) {
      // save_settings 在热键注册失败时会 Err 并回滚——必须把错误呈现给用户，
      // 否则用户点 Finish/Skip 无反应、卡在页面上不知所以。
      console.error('Failed to save onboarding settings:', err)
      setSaveError(err instanceof Error ? err.message : String(err))
      return false
    } finally {
      setSaving(false)
    }
  }, [onSettingsChange, settings])

  const handleSkip = useCallback(async () => {
    const ok = await persistSettings('skipped')
    if (ok) onSkip()
  }, [onSkip, persistSettings])

  const handleSkipAfterLoadFailure = useCallback(async () => {
    setSaving(true)
    setSaveError(null)
    try {
      await updateSettingsCached((current) => ({ ...current, onboardingStatus: 'skipped' }))
      onSettingsChange?.()
      onSkip()
    } catch (err) {
      console.error('Failed to skip onboarding after load error:', err)
      setSaveError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }, [onSettingsChange, onSkip])

  const handleFinish = useCallback(async () => {
    if (!settings) return
    // 正常完成需通过供应商校验；若用户显式「稍后配置模型」，则以 skipped 状态完成——
    // 供应商未验证，标 skipped 比 completed 诚实，且同样不会再次弹引导。
    if (canCompleteOnboarding(settings)) {
      const ok = await persistSettings('completed')
      if (ok) onComplete()
    } else if (providerBypass) {
      const ok = await persistSettings('skipped')
      if (ok) onComplete()
    }
  }, [onComplete, persistSettings, providerBypass, settings])

  const goTo = useCallback((index: number) => {
    const next = Math.max(0, Math.min(index, ONBOARDING_STEPS.length - 1))
    setDirection(next >= stepIndex ? 'forward' : 'back')
    setStepIndex(next)
  }, [stepIndex])

  if (loading) {
    return (
      <div className="onboarding-shell ob settings-embedded kv" data-step="welcome">
        <OnboardingAtmosphere step="welcome" index={0} />
        <div className="ob-loading" role="status" aria-label="Loading">
          <span className="ob-loading-drop" />
          <span className="ob-loading-drop" />
          <span className="ob-loading-drop" />
        </div>
      </div>
    )
  }

  if (!settings) {
    const errorT = i18n.zh
    return (
      <div className="onboarding-shell ob settings-embedded kv" data-step="welcome">
        <OnboardingAtmosphere step="welcome" index={0} />
        <OnboardingTopbar windowControls={windowControls} />
        <div className="ob-error">
          <div className="ob-error-card">
            <span className="ob-eyebrow">Kivio Desktop</span>
            <h2 className="ob-error-title">{errorT.onboardingLoadErrorTitle}</h2>
            <p className="ob-sub">{errorT.onboardingLoadErrorDesc}</p>
            {loadError ? <p className="ob-error-detail">{loadError}</p> : null}
            {saveError ? <p className="ob-error-detail" role="alert">{saveError}</p> : null}
            <div className="ob-error-actions">
              <Button variant="ghost" onClick={() => void handleSkipAfterLoadFailure()} disabled={saving} data-tauri-drag-region="false">
                {errorT.onboardingSkip}
              </Button>
              <Button variant="primary" onClick={() => void loadSettings()} disabled={saving} data-tauri-drag-region="false">
                {errorT.onboardingRetry}
              </Button>
            </div>
          </div>
        </div>
      </div>
    )
  }

  const labels = stepLabels(t)
  const isDone = stepId === 'done'
  const primaryLabel = stepId === 'welcome' ? t.onboardingStart : isDone ? t.onboardingFinish : t.onboardingNext
  const primaryDisabled = saving || (isDone ? !canCompleteOnboarding(settings) && !providerBypass : !canGoNext)

  const handlePrimary = () => {
    if (isDone) {
      void handleFinish()
      return
    }
    goTo(stepIndex + 1)
  }

  const stepCount = ONBOARDING_STEPS.length
  const progress = stepIndex / (stepCount - 1)

  return (
    <div
      className="onboarding-shell ob settings-embedded kv"
      data-step={stepId}
      style={{ '--ob-progress': progress } as CSSProperties}
    >
      <OnboardingAtmosphere step={stepId} index={stepIndex} />

      <OnboardingTopbar
        windowControls={windowControls}
        trailing={!isDone ? (
          <Button variant="ghost" size="sm" onClick={() => setSkipConfirmOpen(true)} data-tauri-drag-region="false">
            {t.onboardingSkip}
          </Button>
        ) : null}
      >
        <nav className="ob-rail" aria-label={t.onboardingProgressLabel} data-tauri-drag-region="false">
          {ONBOARDING_STEPS.map((step, index) => {
            const done = index < stepIndex
            const active = index === stepIndex
            return (
              <div key={step} className={`ob-rail-item${active ? ' is-active' : ''}${done ? ' is-done' : ''}`}>
                {index > 0 ? <span className="ob-rail-link" aria-hidden><span /></span> : null}
                <button
                  type="button"
                  className="ob-rail-step"
                  disabled={!done}
                  aria-current={active ? 'step' : undefined}
                  onClick={() => { if (done) goTo(index) }}
                  data-tauri-drag-region="false"
                >
                  <span className="ob-rail-dot">{done ? <Check size={9} strokeWidth={3.2} /> : null}</span>
                  <span className="ob-rail-label">{labels[step]}</span>
                </button>
              </div>
            )
          })}
        </nav>
      </OnboardingTopbar>

      <main className="ob-stage" data-momo-region>
        <div className="ob-scroll kv-scroll custom-scrollbar" data-tauri-drag-region="false">
          <div key={`${stepId}:${lang}`} className="ob-scene" data-dir={direction}>
            {stepId === 'welcome' ? <WelcomeStep t={t} lang={lang} /> : null}
            {stepId === 'appearance' ? (
              <AppearanceStep t={t} lang={lang} settings={settings} onChange={updateSettings} />
            ) : null}
            {stepId === 'provider' ? (
              <ProviderStep
                t={t}
                lang={lang}
                settings={settings}
                onChange={updateSettings}
                showValidationWarning={!providerValidation.ok}
                validationBypassed={providerBypass}
                onBypassValidation={() => setProviderBypass(true)}
              />
            ) : null}
            {stepId === 'webSearch' ? (
              <WebSearchStep t={t} lang={lang} settings={settings} onChange={updateSettings} />
            ) : null}
            {stepId === 'hotkey' ? (
              <HotkeyStep t={t} settings={settings} onChange={updateSettings} />
            ) : null}
            {isDone ? <DoneStep t={t} lang={lang} settings={settings} /> : null}
          </div>
        </div>
      </main>

      <footer className="ob-foot" data-tauri-drag-region="false">
        {saveError ? (
          <div className="ob-foot-error" role="alert">
            <strong>{t.onboardingSaveError}</strong>
            <span>{saveError}</span>
          </div>
        ) : null}
        <div className="ob-foot-row">
          <div className="ob-foot-start">
            {stepIndex > 0 ? (
              <Button variant="ghost" onClick={() => goTo(stepIndex - 1)} disabled={saving} data-tauri-drag-region="false">
                <ArrowLeft size={14} />
                {t.onboardingBack}
              </Button>
            ) : null}
          </div>
          <div className="ob-foot-meter" aria-hidden>
            <span className="ob-foot-count">{String(stepIndex + 1).padStart(2, '0')}</span>
            <span className="ob-foot-bar"><span /></span>
            <span className="ob-foot-total">{String(stepCount).padStart(2, '0')}</span>
          </div>
          <div className="ob-foot-end">
            {stepId === 'webSearch' ? (
              <Button variant="ghost" onClick={() => goTo(stepIndex + 1)} disabled={saving} data-tauri-drag-region="false">
                {t.onboardingWebSearchSkipStep}
              </Button>
            ) : null}
            <Button variant="primary" onClick={handlePrimary} disabled={primaryDisabled} data-tauri-drag-region="false">
              {primaryLabel}
              {!isDone ? <ArrowRight size={14} /> : null}
            </Button>
          </div>
        </div>
      </footer>

      {skipConfirmOpen ? (
        <div
          className="kv-modal-backdrop kv-modal-backdrop--portal"
          data-tauri-drag-region="false"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setSkipConfirmOpen(false)
          }}
        >
          <div
            className="kv-modal"
            role="dialog"
            aria-modal="true"
            data-tauri-drag-region="false"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <h3 className="kv-modal-title">{t.onboardingSkipConfirmTitle}</h3>
            <p className="kv-row-desc">{t.onboardingSkipConfirmDesc}</p>
            <div className="flex justify-end gap-2 pt-4">
              <Button variant="ghost" onClick={() => setSkipConfirmOpen(false)} data-tauri-drag-region="false">
                {t.cancel}
              </Button>
              <Button
                variant="primary"
                onClick={() => {
                  setSkipConfirmOpen(false)
                  void handleSkip()
                }}
                disabled={saving}
                data-tauri-drag-region="false"
              >
                {t.onboardingSkipConfirm}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  )
}

/** 引导顶栏：整条可拖动窗口；macOS 为红绿灯留出左侧位置，其他平台在右端放窗口三键。 */
function OnboardingTopbar({ children, trailing, windowControls }: {
  children?: ReactNode
  trailing?: ReactNode
  windowControls?: ReactNode
}) {
  return (
    <header className={`ob-top${usesNativeTitlebar ? ' ob-top--mac' : ''}`} data-tauri-drag-region>
      <div className="ob-brand" data-tauri-drag-region>
        <span className="ob-brand-mark" aria-hidden />
        <span className="ob-brand-name">Kivio</span>
      </div>
      <div className="ob-top-center" data-tauri-drag-region>{children}</div>
      <div className="ob-top-end" data-tauri-drag-region>
        {trailing}
        {windowControls}
      </div>
    </header>
  )
}
