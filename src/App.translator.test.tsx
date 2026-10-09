import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'

const mocks = vi.hoisted(() => ({ translate: vi.fn(), commit: vi.fn(), save: vi.fn(), close: vi.fn(), resolveLang: vi.fn() }))
vi.mock('./api/tauri', () => ({
  isTauriRuntime: () => false,
  api: { translateText: mocks.translate, commitTranslation: mocks.commit, resizeWindow: vi.fn(), focusWindow: vi.fn(), closeTranslatorWindow: mocks.close, resolveTranslationTargetLang: mocks.resolveLang },
}))
vi.mock('./api/settingsCache', () => ({
  getSettingsCached: async () => ({ settingsLanguage: 'zh', translatorModel: 'test-model', targetLang: 'auto' }),
  updateSettingsCached: mocks.save,
  subscribeSettings: () => () => {},
}))
vi.mock('./theme/theme', () => ({ applyThemeSettings: vi.fn(), disposeTheme: vi.fn() }))
vi.mock('./chat/ChatWindowHost', () => ({ ChatWindowHost: () => null }))

async function typeAndTranslate(text = '你好') {
  await act(async () => {})
  const input = screen.getByRole('textbox')
  fireEvent.change(input, { target: { value: text } })
  await act(async () => { await vi.advanceTimersByTimeAsync(600) })
  return input
}

function enter(input: HTMLElement) {
  fireEvent.keyDown(input, { key: 'Enter' })
}

describe('input translation', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    window.location.hash = '#translator'
    mocks.translate.mockReset().mockResolvedValue('Hello')
    mocks.commit.mockReset().mockResolvedValue(undefined)
    mocks.close.mockReset().mockResolvedValue(undefined)
    mocks.resolveLang.mockReset().mockResolvedValue('en')
    mocks.save.mockReset().mockImplementation(async mutate => mutate({ targetLang: 'auto' }))
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks() })

  it('shows the automatic target, saves a manual choice and translates the current input again', async () => {
    render(<App />)
    await typeAndTranslate()
    expect(screen.getByRole('button', { name: '目标语言' })).toHaveTextContent('自动 → 英语')
    fireEvent.click(screen.getByRole('button', { name: '目标语言' }))
    await act(async () => { fireEvent.click(screen.getByRole('option', { name: '日语' })) })
    expect(screen.getByRole('button', { name: '目标语言' })).toHaveTextContent('译为日语')
    expect(screen.queryByText('Hello')).not.toBeInTheDocument()
    await act(async () => { await vi.advanceTimersByTimeAsync(600) })
    expect(screen.getByText('Hello')).toBeInTheDocument()
    expect(mocks.translate).toHaveBeenCalledTimes(2)
  })

  it('keeps the saved language and shows an error when a new choice cannot be saved', async () => {
    mocks.save.mockRejectedValueOnce(new Error('Settings unavailable'))
    render(<App />)
    await typeAndTranslate()
    fireEvent.click(screen.getByRole('button', { name: '目标语言' }))
    await act(async () => { fireEvent.click(screen.getByRole('option', { name: '日语' })) })
    expect(screen.getByRole('alert')).toHaveTextContent('Settings unavailable')
    expect(screen.getByRole('button', { name: '目标语言' })).toHaveTextContent('自动')
  })

  it('uses Escape to dismiss the language menu before closing the translator', async () => {
    render(<App />)
    await act(async () => {})
    fireEvent.click(screen.getByRole('button', { name: '目标语言' }))
    await act(async () => { fireEvent.keyDown(document, { key: 'Escape' }) })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(mocks.close).not.toHaveBeenCalled()
    await act(async () => { fireEvent.keyDown(document, { key: 'Escape' }) })
    expect(mocks.close).toHaveBeenCalledTimes(1)
  })

  it('shows a translation error without allowing it to be committed, and recovers on new input', async () => {
    mocks.translate.mockRejectedValueOnce(new Error('Network unavailable'))
    render(<App />)
    const input = await typeAndTranslate()
    expect(screen.getByText('Network unavailable')).toBeInTheDocument()
    await act(async () => { enter(input) })
    expect(mocks.commit).not.toHaveBeenCalled()
    await typeAndTranslate('您好')
    await act(async () => { enter(input) })
    expect(mocks.commit).toHaveBeenCalledWith('Hello')
  })

  it('allows Shift+Enter for a new line and prevents bare Enter from adding one while waiting', async () => {
    render(<App />)
    const input = await typeAndTranslate('你好\n世界')
    expect(input).toHaveValue('你好\n世界')
    expect(fireEvent.keyDown(input, { key: 'Enter', shiftKey: true })).toBe(true)
    expect(mocks.commit).not.toHaveBeenCalled()
    fireEvent.change(input, { target: { value: '新的文字' } })
    expect(fireEvent.keyDown(input, { key: 'Enter' })).toBe(false)
    expect(mocks.commit).not.toHaveBeenCalled()
  })

  it('commits only once while submission is pending', async () => {
    let finish!: () => void
    mocks.commit.mockImplementation(() => new Promise<void>(resolve => { finish = resolve }))
    render(<App />)
    const input = await typeAndTranslate()
    await act(async () => { enter(input); enter(input) })
    expect(mocks.commit).toHaveBeenCalledTimes(1)
    await act(async () => { finish() })
    expect(input).toHaveValue('')
  })

  it('keeps the translation and input available for retry after submission fails', async () => {
    mocks.commit.mockRejectedValueOnce(new Error('Clipboard unavailable'))
    render(<App />)
    const input = await typeAndTranslate()
    await act(async () => { enter(input) })
    expect(screen.getByRole('alert')).toHaveTextContent('Clipboard unavailable')
    expect(input).toHaveValue('你好')
    expect(screen.getByText('Hello')).toBeInTheDocument()
    await act(async () => { enter(input) })
    expect(mocks.commit).toHaveBeenCalledTimes(2)
    expect(input).toHaveValue('')
  })

  it('discards late translations after the input changes and respects IME confirmation', async () => {
    let finish!: (text: string) => void
    mocks.translate.mockImplementationOnce(() => new Promise<string>(resolve => { finish = resolve }))
    render(<App />)
    const input = await typeAndTranslate()
    fireEvent.change(input, { target: { value: '您好' } })
    await act(async () => { finish('Old translation') })
    expect(screen.queryByText('Old translation')).not.toBeInTheDocument()
    await act(async () => { await vi.advanceTimersByTimeAsync(600) })
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    expect(mocks.commit).not.toHaveBeenCalled()
    await act(async () => { enter(input) })
    expect(mocks.commit).toHaveBeenCalledWith('Hello')
  })
})
