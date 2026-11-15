import { open } from '@tauri-apps/plugin-dialog'
import { Toggle, Select, Input, SettingRow, SettingsGroup } from '../components'
import { Button } from '../../components/Button'
import { ModelEffortSelect, ModelPairSelect } from '../ModelPairSelect'
import { PromptField } from '../ScreenshotTranslationSettings'
import type { I18n, Lang } from '../../components/i18n'
import type { Settings as SettingsData } from '../../api/tauri'

interface LensTabProps {
  settings: SettingsData
  t: I18n
  lang: Lang
  /** 当前语言对应的默认 Lens 提示词，用于「恢复默认」。 */
  lensDefaults: { system: string; question: string } | undefined
  onUpdateSettings: (updates: Partial<SettingsData>) => void
  onUpdateLens: (updates: Partial<SettingsData['lens']>) => void
}

/** Lens 标签页。纯展示：状态留在 SettingsShell。 */
export function LensTab({
  settings,
  t,
  lang,
  lensDefaults,
  onUpdateSettings,
  onUpdateLens,
}: LensTabProps) {
  return (
    <>
      <SettingsGroup title={t.lensSection}>
        <SettingRow label={t.enabled}>
          <Toggle
            checked={settings.lens.enabled}
            onChange={(v) => onUpdateLens({ enabled: v })}
          />
        </SettingRow>

        {settings.lens.enabled && (
          <>
            <SettingRow label={lang === 'zh' ? '模型' : 'Model'}>
              <div className="flex flex-wrap items-center justify-end gap-2">
                <ModelPairSelect
                  providerId={settings.lens?.providerId || ''}
                  model={settings.lens?.model || ''}
                  providers={settings.providers}
                  inheritLabel={t.lensLanguageInherit}
                  onChange={(providerId, model) => {
                    onUpdateLens({ providerId, model, thinkingLevel: '' })
                  }}
                />
                <ModelEffortSelect
                  providerId={settings.lens.providerId || settings.translatorProviderId}
                  model={settings.lens.model || settings.translatorModel}
                  providers={settings.providers}
                  value={settings.lens.thinkingLevel ?? (settings.lens.thinkingEnabled === false ? 'off' : '')}
                  onChange={(thinkingLevel) => onUpdateLens({ thinkingLevel })}
                />
              </div>
            </SettingRow>
            <SettingRow label={t.lensResponseLanguage}>
              <Select
                className="kv-ctl"
                value={settings.lens?.defaultLanguage || ''}
                onChange={(v) => onUpdateLens({ defaultLanguage: v })}
                options={[
                  { value: '', label: t.lensLanguageInherit },
                  { value: 'zh', label: '中文' },
                  { value: 'zh-Hant', label: '繁體中文' },
                  { value: 'en', label: 'English' },
                ]}
              />
            </SettingRow>
          </>
        )}
      </SettingsGroup>

      {settings.lens.enabled && (
        <>
          <SettingsGroup title={lang === 'zh' ? '对话' : 'Conversation'}>
            <SettingRow label={t.lensSendToChat}>
              <Toggle
                checked={settings.lens?.sendToChat === true}
                onChange={(v) => onUpdateLens({ sendToChat: v })}
              />
            </SettingRow>
            <SettingRow label={t.lensMessageOrder}>
              <Select
                className="kv-ctl"
                value={settings.lens?.messageOrder ?? 'asc'}
                onChange={(v) => onUpdateLens({ messageOrder: v as 'asc' | 'desc' })}
                options={[
                  { value: 'asc', label: t.lensMessageOrderAsc },
                  { value: 'desc', label: t.lensMessageOrderDesc },
                ]}
              />
            </SettingRow>
            <SettingRow label={t.lensShowCaptureHint}>
              <Toggle
                checked={settings.lens?.showCaptureHint !== false}
                onChange={(v) => onUpdateLens({ showCaptureHint: v })}
              />
            </SettingRow>
          </SettingsGroup>

          <SettingsGroup title={lang === 'zh' ? '截图归档' : 'Screenshot archive'}>
            <SettingRow label={t.imageArchive}>
              <Toggle
                checked={settings.imageArchiveEnabled ?? false}
                onChange={(v) => onUpdateSettings({ imageArchiveEnabled: v })}
              />
            </SettingRow>
            {settings.imageArchiveEnabled && (
              <SettingRow label={t.imageArchivePath} stack>
                <div className="kv-path-row">
                  <Input
                    value={settings.imageArchivePath || ''}
                    onChange={(v) => onUpdateSettings({ imageArchivePath: v })}
                    placeholder={t.imageArchivePathPlaceholder}
                  />
                  <Button
                    onClick={async () => {
                      try {
                        const selected = await open({ directory: true, multiple: false })
                        if (typeof selected === 'string') {
                          onUpdateSettings({ imageArchivePath: selected })
                        }
                      } catch (err) {
                        console.error('Failed to pick directory:', err)
                      }
                    }}
                    data-tauri-drag-region="false"
                  >
                    {t.imageArchiveBrowse}
                  </Button>
                </div>
              </SettingRow>
            )}
          </SettingsGroup>

          <SettingsGroup title={t.customPrompts}>
            <PromptField
              label={t.lensSystemPrompt}
              value={settings.lens?.systemPrompt || ''}
              defaultText={lensDefaults?.system || ''}
              t={t}
              onChange={(v) => onUpdateLens({ systemPrompt: v })}
            />
            <PromptField
              label={t.lensQuestionPrompt}
              value={settings.lens?.questionPrompt || ''}
              defaultText={lensDefaults?.question || ''}
              t={t}
              onChange={(v) => onUpdateLens({ questionPrompt: v })}
            />
          </SettingsGroup>
        </>
      )}
    </>
  )
}
