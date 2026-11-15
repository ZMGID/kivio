import { render, screen, within } from '@testing-library/react'
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

  it('提示词回显 translatorPrompt 而非默认模板', async () => {
    renderTab()
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.translatorPrompt}` }))
    expect(screen.getByDisplayValue('自定义翻译提示')).toBeTruthy()
  })

  it('在弹窗恢复默认并保存后继续跟随内置模板', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.translatorPrompt}` }))
    const dialog = within(screen.getByRole('dialog'))
    await userEvent.click(dialog.getByRole('button', { name: t.restoreDefaultPrompt }))
    expect(dialog.getByRole('textbox')).toHaveValue('默认翻译模板')
    expect(props.onUpdateSettings).not.toHaveBeenCalled()
    await userEvent.click(dialog.getByRole('button', { name: t.save }))
    expect(props.onUpdateSettings).toHaveBeenCalledWith({ translatorPrompt: '' })
  })

  it('键盘焦点留在弹窗内，Esc 关闭后回到编辑按钮', async () => {
    renderTab()
    const edit = screen.getByRole('button', { name: `${t.promptEdit} ${t.translatorPrompt}` })
    await userEvent.click(edit)
    const dialog = within(screen.getByRole('dialog'))
    expect(dialog.getByRole('textbox')).toHaveFocus()
    dialog.getByRole('button', { name: t.save }).focus()
    await userEvent.tab()
    expect(dialog.getAllByRole('button', { name: t.cancel })[0]).toHaveFocus()
    await userEvent.tab({ shift: true })
    expect(dialog.getByRole('button', { name: t.save })).toHaveFocus()
    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(edit).toHaveFocus()
  })
})
