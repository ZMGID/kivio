import type { CSSProperties } from 'react'
import type { Settings } from '../../api/tauri'
import type { I18n, Lang } from '../../components/i18n'
import { Button } from '../../components/Button'
import { ProviderIcon } from '../../components/ModelIcon'
import { KivioBlob } from '../../chat/public/mascot'
import { PROVIDER_PRESETS } from '../../settings/public/modelSelection'
import { OnboardingBlock, OnboardingStepFrame } from '../OnboardingStepFrame'
import { ProviderSetupPanel } from '../ProviderSetupPanel'

type ProviderStepProps = {
  t: I18n
  lang: Lang
  settings: Settings
  onChange: (settings: Settings) => void
  showValidationWarning?: boolean
  onBypassValidation?: () => void
  validationBypassed?: boolean
}

type OrbitBody = { key: string; name: string; baseUrl: string; live: boolean }

/**
 * 「接入」插画：墨墨居中，服务商图标沿两条轨道环绕。已启用模型的服务商点亮并连出光束，
 * 中心状态随启用数量实时更新。
 */
function ProviderOrbitArt({ t, settings }: { t: I18n; settings: Settings }) {
  const live = settings.providers.filter((item) => item.enabled !== false && item.enabledModels.length > 0)
  const modelCount = live.reduce((sum, item) => sum + item.enabledModels.length, 0)
  const bodies: OrbitBody[] = [
    ...live.map((item) => ({ key: item.id, name: item.name, baseUrl: item.baseUrl, live: true })),
    ...PROVIDER_PRESETS
      .filter((preset) => !live.some((item) => item.baseUrl === preset.baseUrl))
      .map((preset) => ({ key: preset.name, name: preset.name, baseUrl: preset.baseUrl, live: false })),
  ].slice(0, 10)
  const inner = bodies.slice(0, 4)
  const outer = bodies.slice(4)
  const ready = modelCount > 0

  const ring = (items: OrbitBody[], className: string) => (
    <div className={`ob-orbit-ring ${className}`}>
      {items.map((item, index) => (
        <span
          key={item.key}
          className={`ob-orbit-body${item.live ? ' is-live' : ''}`}
          style={{ '--a': `${(360 / items.length) * index}deg` } as CSSProperties}
        >
          <span className="ob-orbit-chip"><ProviderIcon name={item.name} baseUrl={item.baseUrl} size={16} /></span>
        </span>
      ))}
    </div>
  )

  return (
    <div className={`ob-orbit${ready ? ' is-ready' : ''}`}>
      <span className="ob-orbit-track ob-orbit-track--inner" />
      <span className="ob-orbit-track ob-orbit-track--outer" />
      {ring(inner, 'ob-orbit-ring--inner')}
      {ring(outer, 'ob-orbit-ring--outer')}
      <span className="ob-orbit-pulse" />
      <span className="ob-orbit-core"><KivioBlob size={60} mood={ready ? 'idle' : 'wait'} /></span>
      <span className="ob-orbit-status">
        <i />
        {ready ? t.onboardingProviderOrbitReady.replace('{n}', String(modelCount)) : t.onboardingProviderOrbitIdle}
      </span>
    </div>
  )
}

export function ProviderStep({
  t,
  lang,
  settings,
  onChange,
  showValidationWarning = false,
  onBypassValidation,
  validationBypassed = false,
}: ProviderStepProps) {
  return (
    <OnboardingStepFrame
      index={2}
      eyebrow={t.onboardingStepProvider}
      title={t.onboardingProviderTitle}
      subtitle={t.onboardingProviderDesc}
      art={<ProviderOrbitArt t={t} settings={settings} />}
    >
      <ProviderSetupPanel t={t} lang={lang} settings={settings} onChange={onChange} />
      {showValidationWarning ? (
        <OnboardingBlock order={3} className="ob-callout">
          <p className="onboarding-panel-note">{t.onboardingProviderRequired}</p>
          {!validationBypassed && onBypassValidation ? (
            <Button onClick={onBypassValidation} data-tauri-drag-region="false">
              {t.onboardingProviderContinueAnyway}
            </Button>
          ) : null}
        </OnboardingBlock>
      ) : null}
    </OnboardingStepFrame>
  )
}
