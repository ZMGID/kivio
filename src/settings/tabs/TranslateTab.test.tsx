import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { TranslateTab } from './TranslateTab'
import { makeSettings, makeProvider } from './testFixtures'
import { i18n } from '../../components/i18n'

const t = i18n.zh

/** 回归重点：目标语言 / 模型对 / 提示词各写对 settings 顶层字段。 */
function renderTab() {
  const props = {
    settings: makeSettings({
      providers: [makeProvider()],
      targetLang: 'en',
      readingTargetLang: 'zh',
      translatorPrompt: '自定义翻译提示',
    }),
    t,
    lang: 'zh' as const,
    defaultPrompts: {
      translationTemplate: '默认翻译模板',
      lensPrompts: { zh: { system: '', question: '' }, en: { system: '', question: '' } },
    } as never,
    onUpdateSettings: vi.fn(),
  }
  render(<TranslateTab {...props} />)
  return props
}

describe('TranslateTab', () => {
  it('渲染输出 / 模型 / 提示词三组', () => {
    renderTab()
    expect(screen.getByText('输入翻译输出语言')).toBeTruthy()
    expect(screen.getByText('阅读目标语言')).toBeTruthy()
    expect(screen.getByText(t.selectModelPair)).toBeTruthy()
    expect(screen.getByText(t.translatorPrompt)).toBeTruthy()
  })

  // Select 是自绘组件（button + 弹出菜单），不是原生 <select>，故点开后选条目。
  it('目标语言下拉写 targetLang', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: '输入翻译输出语言' }))
    await userEvent.click(screen.getByText('西班牙语'))
    expect(props.onUpdateSettings).toHaveBeenCalledWith({ targetLang: 'es' })
  })

  it('阅读语言独立保存，不提供中英互译的自动选项', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: '阅读目标语言' }))
    expect(screen.queryByText(t.langAuto)).toBeNull()
    await userEvent.click(screen.getByText('泰语'))
    expect(props.onUpdateSettings).toHaveBeenCalledTimes(1)
    expect(props.onUpdateSettings).toHaveBeenCalledWith({ readingTargetLang: 'th' })
  })

  it('提示词回显 translatorPrompt 而非默认模板', () => {
    renderTab()
    expect(screen.getByDisplayValue('自定义翻译提示')).toBeTruthy()
  })
})
