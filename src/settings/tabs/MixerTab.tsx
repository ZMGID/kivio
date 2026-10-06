import { useEffect, useState } from 'react'
import { api, type SubAgentModelSelection } from '../../api/tauri'
import { Select } from '../public/controls'
import { Toggle, SettingRow, SettingsGroup } from '../components'
import { Button } from '../../components/Button'
import { ModelPairSelect } from '../ModelPairSelect'
import { PromptField } from '../ScreenshotTranslationSettings'
import { resolveModelInfo } from '../../data/modelMatching'
import type { I18n, Lang } from '../../components/i18n'
import type { Settings as SettingsData, ChatToolsConfig } from '../../api/tauri'

interface MixerTabProps {
  settings: SettingsData
  t: I18n
  lang: Lang
  chatTools: ChatToolsConfig
  /** 是否已配好聊天供应商；未配好时显示引导文案。 */
  hasChatProvider: boolean
  defaultPromptOptimize?: string
  onUpdateDefaultModel: (
    key: keyof SettingsData['defaultModels'],
    providerId: string,
    model: string,
  ) => void
  onUpdateChatTools: (updates: Partial<ChatToolsConfig> | ((current: ChatToolsConfig) => Partial<ChatToolsConfig>)) => void
  onUpdateChat: (updates: Partial<NonNullable<SettingsData['chat']>>) => void
}

/** 模型分工（Mixer）标签页。纯展示：状态留在 SettingsShell。 */
export function MixerTab({
  settings,
  t,
  lang,
  chatTools,
  hasChatProvider,
  defaultPromptOptimize = '',
  onUpdateDefaultModel,
  onUpdateChatTools,
  onUpdateChat,
}: MixerTabProps) {
  return (
    <>
      <SettingsGroup title={t.mixerSection}>
        <div className="mb-3 flex items-start justify-between gap-3">
          {t.mixerSectionHint ? (
            <p className="kv-row-desc max-w-[560px]">{t.mixerSectionHint}</p>
          ) : <span />}
          <Button
            size="sm"
            className="shrink-0"
            onClick={() => {
              onUpdateDefaultModel('vision', '', '')
              onUpdateDefaultModel('videoAnalysis', '', '')
              onUpdateDefaultModel('titleSummary', '', '')
              onUpdateDefaultModel('compression', '', '')
              onUpdateDefaultModel('imageGeneration', '', '')
              onUpdateDefaultModel('promptOptimize', '', '')
            }}
            data-tauri-drag-region="false"
          >
            {t.mixerResetAuto}
          </Button>
        </div>
        <SettingRow
          label={t.auxiliaryVisionModel}
        >
          <ModelPairSelect
            providerId={settings.defaultModels.vision.providerId || ''}
            model={settings.defaultModels.vision.model || ''}
            providers={settings.providers}
            inheritLabel={t.mixerAutoVisionModel}
            filterModel={(provider, model) =>
              resolveModelInfo(model, provider.modelOverrides, provider).capabilities?.vision === true
            }
            onChange={(providerId, model) => {
              onUpdateDefaultModel('vision', providerId, model)
            }}
          />
        </SettingRow>
        <SettingRow label={t.videoAnalysisModel} description={t.videoAnalysisModelHint}>
          <ModelPairSelect
            providerId={settings.defaultModels.videoAnalysis?.providerId || ''}
            model={settings.defaultModels.videoAnalysis?.model || ''}
            providers={settings.providers}
            inheritLabel={t.mixerAutoVisionModel}
            offOption={{
              label: lang === 'zh' ? '关闭' : 'Off',
              selected: settings.chat?.videoAnalysisEnabled === false,
              onSelect: () => onUpdateChat({ videoAnalysisEnabled: false }),
            }}
            filterModel={(provider, model) =>
              resolveModelInfo(model, provider.modelOverrides, provider).capabilities?.videoInput === true
            }
            onChange={(providerId, model) => {
              onUpdateDefaultModel('videoAnalysis', providerId, model)
              onUpdateChat({ videoAnalysisEnabled: true })
            }}
          />
        </SettingRow>
        <SettingRow
          label={t.defaultTitleSummaryModel}
        >
          <ModelPairSelect
            providerId={settings.defaultModels.titleSummary.providerId || ''}
            model={settings.defaultModels.titleSummary.model || ''}
            providers={settings.providers}
            inheritLabel={t.mixerAutoModel}
            onChange={(providerId, model) => {
              onUpdateDefaultModel('titleSummary', providerId, model)
            }}
          />
        </SettingRow>
        <SettingRow
          label={t.defaultCompressionModel}
        >
          <ModelPairSelect
            providerId={settings.defaultModels.compression.providerId || ''}
            model={settings.defaultModels.compression.model || ''}
            providers={settings.providers}
            inheritLabel={t.mixerAutoModel}
            onChange={(providerId, model) => {
              onUpdateDefaultModel('compression', providerId, model)
            }}
          />
        </SettingRow>
        <SettingRow
          label={t.defaultImageGenerationModel}
          description={t.defaultImageGenerationModelHint}
        >
          <ModelPairSelect
            providerId={settings.defaultModels.imageGeneration.providerId || ''}
            model={settings.defaultModels.imageGeneration.model || ''}
            providers={settings.providers}
            inheritLabel={t.mixerNoImageGenerationModel}
            filterModel={(provider, model) =>
              resolveModelInfo(model, provider.modelOverrides, provider).capabilities?.imageGeneration === true
            }
            onChange={(providerId, model) => {
              onUpdateDefaultModel('imageGeneration', providerId, model)
            }}
          />
        </SettingRow>
        <SettingRow
          label={t.defaultPromptOptimizeModel}
          description={t.defaultPromptOptimizeModelHint}
        >
          <ModelPairSelect
            providerId={settings.defaultModels.promptOptimize?.providerId || ''}
            model={settings.defaultModels.promptOptimize?.model || ''}
            providers={settings.providers}
            inheritLabel={t.mixerAutoModel}
            onChange={(providerId, model) => {
              onUpdateDefaultModel('promptOptimize', providerId, model)
            }}
          />
        </SettingRow>
        <PromptField
          label={t.promptOptimizePrompt}
          description={t.promptOptimizePromptHint}
          value={settings.chat?.promptOptimizePrompt || ''}
          defaultText={defaultPromptOptimize}
          restoreLabel={t.restoreDefaultPrompt}
          onChange={(promptOptimizePrompt) => onUpdateChat({ promptOptimizePrompt })}
        />
        {!hasChatProvider && (
          <p className="kv-row-desc px-0 pb-2">
            {lang === 'zh' ? '请先在「模型」中添加并配置供应商。' : 'Add and configure a provider under Models first.'}
          </p>
        )}
      </SettingsGroup>

      <SettingsGroup title={t.mixerSubAgentSection}>
        <p className="kv-row-desc mb-3">
          {lang === 'zh'
            ? '为子代理角色一起选择模型与推理强度。未单独配置的角色跟随 TASK；TASK 跟随主对话。已有子代理继续使用启动时的配置。'
            : 'Choose a model and reasoning effort for each role. Unassigned roles follow TASK, then the parent chat. Existing agents keep their launch configuration.'}
        </p>
        {([
          ['task', 'TASK', lang === 'zh' ? '通用、编码代理' : 'General-purpose and coding agents'],
          ['smol', 'SMOL', lang === 'zh' ? '研究代理 · 只读搜索与调查' : 'Research agents · read-only investigation'],
          ['slow', 'SLOW', lang === 'zh' ? '审查代理 · 分析正确性与风险' : 'Review agents · correctness and risk'],
        ] as const).map(([role, label, description]) => (
          <SettingRow key={role} label={label} description={description}>
            <SubAgentRoleSelect
              value={chatTools.subAgentModels?.[role] ?? { providerId: '', model: '' }}
              providers={settings.providers}
              lang={lang}
              role={label}
              inheritLabel={role === 'task' ? t.mixerFollowChatModel : (lang === 'zh' ? '跟随 TASK' : 'Follow TASK')}
              onChange={(selection) => onUpdateChatTools((current) => ({
                subAgentModels: { ...current.subAgentModels, [role]: selection },
              }))}
            />
          </SettingRow>
        ))}
      </SettingsGroup>

      <SettingsGroup title={t.mixerAdvisorSection}>
        <SettingRow
          label={t.defaultAdvisorModel}
          description={t.defaultAdvisorModelHint}
        >
          <Toggle
            checked={Boolean(settings.defaultModels.advisor.providerId)}
            onChange={(on) => {
              if (on) {
                // 开启：若尚未选过模型，默认落到第一个可用供应商的首个模型，用户可再改。
                if (!settings.defaultModels.advisor.providerId) {
                  const p = settings.providers.find(
                    (pp) => pp.enabled && (pp.enabledModels?.length ?? 0) > 0,
                  )
                  onUpdateDefaultModel('advisor', p?.id ?? '', p?.enabledModels[0] ?? '')
                }
              } else {
                onUpdateDefaultModel('advisor', '', '')
              }
            }}
          />
        </SettingRow>
        {Boolean(settings.defaultModels.advisor.providerId) && (
          <SettingRow label={lang === 'zh' ? '顾问模型' : 'Advisor model'}>
            <ModelPairSelect
              providerId={settings.defaultModels.advisor.providerId || ''}
              model={settings.defaultModels.advisor.model || ''}
              providers={settings.providers}
              onChange={(providerId, model) => {
                onUpdateDefaultModel('advisor', providerId, model)
              }}
            />
          </SettingRow>
        )}
      </SettingsGroup>
    </>
  )
}

/** Both selectors edit one assignment; changing models clears the old model's effort. */
function SubAgentRoleSelect({ value, providers, lang, role, inheritLabel, onChange }: {
  value: SubAgentModelSelection
  providers: SettingsData['providers']
  lang: Lang
  role: string
  inheritLabel: string
  onChange: (value: SubAgentModelSelection) => void
}) {
  const [capability, setCapability] = useState<{ key: string; levels: string[] } | null>(null)
  const key = JSON.stringify([value.providerId, value.model, providers])
  useEffect(() => {
    let active = true
    if (value.providerId && value.model) {
      void api.reasoningEffortsForModel(value.model, value.providerId).then((levels) => {
        if (active) setCapability({ key, levels })
      }).catch(() => {
        if (active) setCapability(null)
      })
    }
    return () => { active = false }
  }, [key, value.providerId, value.model])
  const levels = capability?.key === key ? capability.levels : null
  const selected = Boolean(value.providerId && value.model)
  const options = [
    { value: '', label: lang === 'zh' ? '模型设置' : 'Model setting' },
    ...(levels?.length ? [
      { value: 'off', label: 'Off' },
      ...levels.map((level) => ({ value: level, label: level })),
    ] : []),
  ]
  if (value.thinkingLevel && !options.some((option) => option.value === value.thinkingLevel)) {
    options.push({ value: value.thinkingLevel, label: value.thinkingLevel })
  }
  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      <ModelPairSelect
        providerId={value.providerId}
        model={value.model}
        providers={providers}
        inheritLabel={inheritLabel}
        onChange={(providerId, model) => onChange({ providerId, model, thinkingLevel: null })}
      />
      {selected && (
        <Select
          className="w-32"
          ariaLabel={`${role} ${lang === 'zh' ? '推理强度' : 'reasoning effort'}`}
          value={value.thinkingLevel ?? ''}
          options={options}
          disabled={!levels?.length}
          title={levels?.length === 0 ? (lang === 'zh' ? '此模型不支持调整推理强度' : 'This model has no adjustable reasoning effort') : undefined}
          onChange={(thinkingLevel) => onChange({ ...value, thinkingLevel: thinkingLevel || null })}
        />
      )}
    </div>
  )
}
