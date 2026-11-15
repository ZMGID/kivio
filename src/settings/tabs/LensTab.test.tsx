import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { LensTab } from './LensTab'
import { makeSettings, makeProvider } from './testFixtures'
import { i18n } from '../../components/i18n'
import { api } from '../../api/tauri'

vi.mock('../../api/tauri', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../api/tauri')>(),
  api: { thinkingCapabilitiesForModel: vi.fn().mockResolvedValue({ levels: ['low', 'high'], offMode: 'supported' }) },
}))

const t = i18n.zh

/**
 * 回归重点：
 *   1. enabled 总开关的三层嵌套条件（关掉时下面各组必须整体消失）
 *   2. onUpdateLens vs onUpdateSettings 分流 —— 图片归档写顶层 settings，其余写 lens
 */
function renderTab(settingsOverrides: Record<string, unknown> = {}) {
  const props = {
    settings: makeSettings({
      providers: [makeProvider()],
      lens: { enabled: true, hotkey: '', providerId: 'p1', model: 'gpt-4o' },
      ...settingsOverrides,
    } as never),
    t,
    lang: 'zh' as const,
    lensDefaults: { system: '默认系统提示', question: '默认问题提示' },
    onUpdateSettings: vi.fn(),
    onUpdateLens: vi.fn(),
  }
  render(<LensTab {...props} />)
  return props
}

describe('LensTab', () => {

  it('发送到客户端默认关闭，保留用户显式开启的选择', async () => {
    const props = renderTab()
    const row = screen.getByText(t.lensSendToChat).closest('.kv-row')
    if (!(row instanceof HTMLElement)) throw new Error('Missing handoff row')
    const toggle = within(row).getByRole('switch')
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    await userEvent.click(toggle)
    expect(props.onUpdateLens).toHaveBeenCalledWith({ sendToChat: true })
    cleanup()
    renderTab({ lens: { enabled: true, hotkey: '', sendToChat: true } })
    const enabledRow = screen.getByText(t.lensSendToChat).closest('.kv-row')
    if (!(enabledRow instanceof HTMLElement)) throw new Error('Missing handoff row')
    expect(within(enabledRow).getByRole('switch')).toHaveAttribute('aria-checked', 'true')
  })

  it('继承翻译模型的能力并保存 effort，切换模型清除旧档位', async () => {
    const props = renderTab({ lens: { enabled: true, hotkey: '', providerId: '', model: '', thinkingLevel: 'high' } })
    const effort = await screen.findByRole('button', { name: t.reasoningEffort })
    expect(api.thinkingCapabilitiesForModel).toHaveBeenCalledWith('gpt-4o', 'p1')
    expect(screen.queryByText(t.lensStreamEnabled)).toBeNull()
    expect(screen.queryByText(t.lensThinkingEnabled)).toBeNull()
    await userEvent.click(effort)
    await userEvent.click(screen.getByRole('option', { name: 'low' }))
    expect(props.onUpdateLens).toHaveBeenCalledWith({ thinkingLevel: 'low' })
    const modelRow = effort.closest('.kv-row')
    if (!(modelRow instanceof HTMLElement)) throw new Error('Missing model row')
    await userEvent.click(within(modelRow).getByRole('button', { name: t.lensLanguageInherit }))
    await userEvent.click(screen.getByRole('option', { name: /gpt-4o/ }))
    expect(props.onUpdateLens).toHaveBeenCalledWith({ providerId: 'p1', model: 'gpt-4o', thinkingLevel: '' })
  })

  it('enabled 关闭时隐藏下游全部分组（三层嵌套条件）', () => {
    renderTab()
    expect(screen.getByText(t.lensResponseLanguage)).toBeTruthy()
    expect(screen.getByText(t.customPrompts)).toBeTruthy()
    cleanup()
    renderTab({ lens: { enabled: false, hotkey: '' } })
    // 总开关所在组仍在
    expect(screen.getByText(t.lensSection)).toBeTruthy()
    // 下游各组必须整体消失
    expect(screen.queryAllByText(t.imageArchive)).toHaveLength(0)
    expect(screen.queryByText(t.customPrompts)).toBeNull()
    expect(screen.queryByText(t.lensResponseLanguage)).toBeNull()
  })

  it('总开关写 lens.enabled 而非顶层 settings', async () => {
    const props = renderTab()
    const toggles = screen.getAllByRole('switch')
    await userEvent.click(toggles[0])
    expect(props.onUpdateLens).toHaveBeenCalledWith({ enabled: false })
    expect(props.onUpdateSettings).not.toHaveBeenCalled()
  })

  it('图片归档开关写顶层 settings（不是 lens）', async () => {
    const props = renderTab()
    const toggles = screen.getAllByRole('switch')
    // 顺序：enabled / sendToChat / showCaptureHint / imageArchive
    await userEvent.click(toggles[toggles.length - 1])
    expect(props.onUpdateSettings).toHaveBeenCalledWith({ imageArchiveEnabled: true })
  })

  it('imageArchiveEnabled 开启后才显示路径行', () => {
    renderTab()
    expect(screen.queryByText(t.imageArchivePath)).toBeNull()
    renderTab({ imageArchiveEnabled: true })
    expect(screen.getByText(t.imageArchivePath)).toBeTruthy()
  })

  it('提示词的默认值来自 lensDefaults（system / question 不串）', async () => {
    renderTab()
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.lensSystemPrompt}` }))
    expect(screen.getByRole('dialog').querySelector('textarea')).toHaveValue('默认系统提示')
    await userEvent.keyboard('{Escape}')
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.lensQuestionPrompt}` }))
    expect(screen.getByRole('dialog').querySelector('textarea')).toHaveValue('默认问题提示')
  })
})
