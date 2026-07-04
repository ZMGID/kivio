import { describe, expect, it } from 'vitest'
import { i18n, normalizeLang, promptLangKey } from './i18n'

describe('i18n language normalization', () => {
  it('normalizes traditional Chinese aliases to zh-TW', () => {
    expect(normalizeLang('zh-TW')).toBe('zh-TW')
    expect(normalizeLang('zh-Hant')).toBe('zh-TW')
    expect(promptLangKey('zh-Hant')).toBe('zh-TW')
  })

  it('defaults missing and unknown language codes to Traditional Chinese', () => {
    expect(normalizeLang()).toBe('zh-TW')
    expect(normalizeLang('bogus')).toBe('zh-TW')
    expect(normalizeLang('zh')).toBe('zh')
    expect(normalizeLang('zh-CN')).toBe('zh')
  })

  it('keeps a complete Traditional Chinese locale table', () => {
    const zhKeys = Object.keys(i18n.zh).sort()
    expect(Object.keys(i18n['zh-TW']).sort()).toEqual(zhKeys)
    expect(Object.keys(i18n.en).sort()).toEqual(zhKeys)
    expect(i18n['zh-TW'].settings).toBe('設定')
    expect(i18n['zh-TW'].systemPrompt).toBe('系統提示詞')
  })

  it('uses Taiwan Traditional Chinese terminology for the zh-TW table', () => {
    const text = Object.values(i18n['zh-TW']).join('\n')
    for (const term of [
      '保存',
      '默認',
      '網絡',
      '搜索',
      '添加',
      '支持',
      '屏幕',
      '視頻',
      '鏈接',
      '檢測',
      '服務器',
      '接口',
      '密鑰',
      '消息',
      '文本',
      '響應語言',
      '服務商',
      '提供商',
      '窗口',
      '重置',
      '登錄',
      '本地',
      '全局',
      '配置',
      '界面',
      '聯網',
      '全屏',
      '信息',
      '剪貼板',
      '認證方式',
      '概覽',
      '協議',
      '上報',
      '計劃',
      '高級',
      '構建',
      '模板',
      '驅動',
      '條已壓縮',
    ]) {
      expect(text).not.toContain(term)
    }
    expect(i18n['zh-TW'].save).toBe('儲存')
    expect(i18n['zh-TW'].defaultModelsSection).toBe('預設模型')
    expect(i18n['zh-TW'].tabWebSearch).toBe('網路搜尋')
  })
})
