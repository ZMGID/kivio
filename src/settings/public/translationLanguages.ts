import type { I18n } from '../../components/i18n'

/** 输入翻译设置与浮窗共用语言选项；自动目标的解析仍归后端。 */
export function getTranslationLanguageOptions(t: I18n) {
  return [
    { value: 'auto', label: t.langAuto },
    { value: 'en', label: t.langEn },
    { value: 'zh', label: t.langZh },
    { value: 'zh-Hant', label: t.langZhTw },
    { value: 'ja', label: t.langJa },
    { value: 'ko', label: t.langKo },
    { value: 'fr', label: t.langFr },
    { value: 'de', label: t.langDe },
  ]
}
