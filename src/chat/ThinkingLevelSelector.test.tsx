import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ThinkingLevelSelector } from './ThinkingLevelSelector'

const { thinkingCapabilitiesForModel } = vi.hoisted(() => ({
  thinkingCapabilitiesForModel: vi.fn(),
}))

// api 在 jsdom 无 Tauri 环境，mock 成确定值；等级清单走兜底也是同样结果。
vi.mock('../api/tauri', () => ({
  api: {
    getSettings: () => Promise.resolve({ providers: [] }),
    thinkingCapabilitiesForModel,
  },
}))

describe('ThinkingLevelSelector', () => {
  beforeEach(() => {
    thinkingCapabilitiesForModel.mockReset()
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: ['low', 'medium', 'high'], offMode: 'supported' })
  })

  it('OAuth 档位模型返回空能力时隐藏旋钮，不回写残留的会话档位', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'unsupported' })
    const onChange = vi.fn()
    await act(async () => {
      render(<ThinkingLevelSelector value="high" currentProviderId="antigravity-oauth" currentModel="gemini-3.8-flash-low" onChange={onChange} />)
    })
    expect(thinkingCapabilitiesForModel).toHaveBeenCalledWith('gemini-3.8-flash-low', 'antigravity-oauth')
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })


  it('选择某一档回调原始等级值', async () => {
    const onChange = vi.fn()
    render(
      <ThinkingLevelSelector
        value="high"
        currentProviderId="p1"
        currentModel="m1"
        onChange={onChange}
      />,
    )
    await waitFor(() => expect(screen.getByRole('button')).toBeEnabled())
    act(() => {
      fireEvent.click(screen.getByRole('button'))
    })
    await waitFor(() => expect(screen.getByRole('button')).toBeEnabled())
    act(() => {
      fireEvent.click(screen.getByText('Off'))
    })
    expect(onChange).toHaveBeenCalledWith('off')
  })

  it('能力列表加载完成前不会把 xhigh 回写成 high', async () => {
    let resolveLevels!: (levels: { levels: string[]; offMode: 'supported' }) => void
    thinkingCapabilitiesForModel.mockImplementationOnce(() => new Promise((resolve) => {
      resolveLevels = resolve
    }))
    const onChange = vi.fn()

    render(
      <ThinkingLevelSelector
        value="xhigh"
        currentProviderId="p1"
        currentModel="gpt-5.6-sol"
        onChange={onChange}
      />,
    )

    expect(onChange).not.toHaveBeenCalled()

    await act(async () => {
      resolveLevels({ levels: ['low', 'medium', 'high', 'xhigh'], offMode: 'supported' })
    })
    expect(screen.getByRole('button')).toHaveTextContent('XHigh')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('没有强度档位但支持开关时仍可关闭并重新开启', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'supported' })
    const onChange = vi.fn()
    await act(async () => { render(<ThinkingLevelSelector value="high" currentProviderId="p1" currentModel="kimi-k2.6" onChange={onChange} />) })
    expect(screen.getByRole('button')).toHaveTextContent('On')
    fireEvent.click(screen.getByRole('button'))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Off' }))
    expect(onChange).toHaveBeenCalledWith('off')
    fireEvent.click(screen.getByRole('button'))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'On' }))
    expect(onChange).toHaveBeenLastCalledWith('high')
  })
  it.each([
    { levels: ['low', 'medium', 'high', 'xhigh', 'max'], expected: 'high' },
    { levels: ['low', 'max'], expected: 'max' },
  ])('reconciles stale Off to a supported effort: $expected', async ({ levels, expected }) => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels, offMode: 'unsupported' })
    const onChange = vi.fn()
    await act(async () => { render(<ThinkingLevelSelector value="off" currentProviderId="p1" currentModel="always-thinking" onChange={onChange} />) })
    expect(onChange).toHaveBeenCalledWith(expected)
    fireEvent.click(screen.getByRole('button'))
    expect(screen.queryByRole('menuitemradio', { name: 'Off' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Low' }))
    expect(onChange).toHaveBeenLastCalledWith('low')
  })

  it('repairs stale Off without showing a control for a model with no effort choices', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'unsupported' })
    const onChange = vi.fn()
    await act(async () => { render(<ThinkingLevelSelector value="off" currentProviderId="p1" currentModel="always-on" onChange={onChange} />) })
    expect(onChange).toHaveBeenCalledWith('high')
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })


  it('换模型等待新能力时不使用旧档位回写选择', async () => {
    const onChange = vi.fn()
    const props = { currentProviderId: 'p1', onChange }
    let resolveNext!: (caps: { levels: string[]; offMode: 'supported' }) => void
    const { rerender } = render(<ThinkingLevelSelector {...props} value="high" currentModel="old" />)
    await waitFor(() => expect(screen.getByRole('button')).toBeEnabled())
    thinkingCapabilitiesForModel.mockImplementationOnce(() => new Promise(resolve => { resolveNext = resolve }))
    await act(async () => { rerender(<ThinkingLevelSelector {...props} value="xhigh" currentModel="new" />) })
    expect(onChange).not.toHaveBeenCalled()
    expect(screen.getByRole('button')).toBeDisabled()
    await act(async () => { resolveNext({ levels: ['low', 'high', 'xhigh'], offMode: 'supported' }) })
    expect(screen.getByRole('button')).toHaveTextContent('XHigh')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('Esc 关闭已展开的下拉', async () => {
    render(
      <ThinkingLevelSelector
        value="high"
        currentProviderId="p1"
        currentModel="m1"
        onChange={() => {}}
      />,
    )
    await waitFor(() => expect(screen.getByRole('button')).toBeEnabled())
    act(() => {
      fireEvent.click(screen.getByRole('button'))
    })
    expect(screen.getByText('Off')).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button')).toBeEnabled())
    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' })
    })
    expect(screen.queryByText('Off')).not.toBeInTheDocument()
  })
})
