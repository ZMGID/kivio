import type { I18n } from '../../components/i18n'

/** 阅读翻译、输入翻译设置与浮窗共用语言选项；语言解析归后端。 */
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
    { value: 'es', label: t.langEs },
    { value: 'pt', label: t.langPt },
    { value: 'it', label: t.langIt },
    { value: 'ru', label: t.langRu },
    { value: 'ar', label: t.langAr },
    { value: 'hi', label: t.langHi },
    { value: 'th', label: t.langTh },
    { value: 'vi', label: t.langVi },
    { value: 'id', label: t.langId },
    { value: 'ms', label: t.langMs },
    { value: 'tr', label: t.langTr },
    { value: 'nl', label: t.langNl },
    { value: 'pl', label: t.langPl },
    { value: 'uk', label: t.langUk },
    { value: 'sv', label: t.langSv },
    { value: 'da', label: t.langDa },
    { value: 'no', label: t.langNo },
    { value: 'fi', label: t.langFi },
  ]
}
