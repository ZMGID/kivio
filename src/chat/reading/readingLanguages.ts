export const readingLanguages = [
  { value: 'en', label: '英语' }, { value: 'zh', label: '简体中文' },
  { value: 'ja', label: '日语' }, { value: 'de', label: '德语' },
  { value: 'fr', label: '法语' }, { value: 'es', label: '西班牙语' }, { value: 'ko', label: '韩语' },
]
export const readingLanguageName = (language?: string) => readingLanguages.find(item => item.value === language)?.label ?? language ?? ''
