import { getTranslationLanguageOptions } from '../public/translationLanguages'
import { Select, SettingRow, SettingsGroup } from '../components'
import { ModelPairSelect } from '../ModelPairSelect'
import { PromptField } from '../ScreenshotTranslationSettings'
import type { I18n, Lang } from '../../components/i18n'
import type { Settings as SettingsData, DefaultPromptTemplates } from '../../api/tauri'

interface TranslateTabProps {
  settings: SettingsData
  t: I18n
  lang: Lang
  defaultPrompts: DefaultPromptTemplates | null
  onUpdateSettings: (updates: Partial<SettingsData>) => void
}

/**
 * 划词翻译设置（翻译标签页的上半部分）。
 *
 * 下半部分「截图翻译」是既有的 ScreenshotTranslationSettings（16 个 props），
 * 仍由 SettingsShell 直接渲染 —— 一并包进来会让本组件涨到 19 个 props，
 * 反而比留在 shell 里更难维护。
 */
export function TranslateTab({
  settings,
  t,
  lang,
  defaultPrompts,
  onUpdateSettings,
}: TranslateTabProps) {
  const readingLabel = lang === 'zh' ? '阅读目标语言' : 'Reading language'
  const outputLabel = lang === 'zh' ? '输入翻译输出语言' : 'Input translation language'
  return (
    <>
      <div className="kv-section-title">{lang === 'zh' ? '翻译语言' : 'Translation languages'}</div>
      <SettingsGroup>
        <SettingRow label={readingLabel} description={lang === 'zh' ? '你想看的语言，用于划词、截图和替换翻译' : 'The language you read, used for selected text, screenshots, and in-place translation'}>
          <Select
            className="w-40"
            ariaLabel={readingLabel}
            value={settings.readingTargetLang}
            onChange={(v) => onUpdateSettings({ readingTargetLang: v })}
            options={getTranslationLanguageOptions(t).filter(option => option.value !== 'auto')}
          />
        </SettingRow>
        <SettingRow label={outputLabel} description={lang === 'zh' ? '你准备发送的语言，用于输入翻译浮窗；自动仅用于中英互译' : 'The language you send from the input translator; Auto switches between Chinese and English'}>
          <Select
            className="w-40"
            ariaLabel={outputLabel}
            value={settings.targetLang}
            onChange={(v) => onUpdateSettings({ targetLang: v })}
            options={getTranslationLanguageOptions(t)}
          />
        </SettingRow>
      </SettingsGroup>

      <div className="kv-section-title">{t.tabTranslate}</div>
      <SettingsGroup title={t.sectionModel}>
        <SettingRow label={t.selectModelPair}>
          <ModelPairSelect
            providerId={settings.translatorProviderId}
            model={settings.translatorModel}
            providers={settings.providers}
            onChange={(providerId, model) => {
              onUpdateSettings({ translatorProviderId: providerId, translatorModel: model })
            }}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title={t.sectionPrompt}>
        <PromptField
          label={t.translatorPrompt}
          description={t.translatorPromptHint}
          value={settings.translatorPrompt || ''}
          defaultText={defaultPrompts?.translationTemplate || ''}
          restoreLabel={t.restoreDefaultPrompt}
          onChange={(v) => onUpdateSettings({ translatorPrompt: v })}
        />
      </SettingsGroup>
    </>
  )
}
