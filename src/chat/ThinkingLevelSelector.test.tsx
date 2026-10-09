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

  it('value=null 时按默认档显示 High（不再有「跟随全局」）', async () => {
    render(
      <ThinkingLevelSelector
        value={null}
        currentProviderId="p1"
        currentModel="m1"
        onChange={() => {}}
      />,
    )
    await waitFor(() => expect(screen.getByRole('button')).toBeEnabled())
    expect(screen.getByRole('button')).toHaveTextContent('High')
  })

  it('下拉项为英文标签且不含「跟随全局」', async () => {
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
    expect(screen.queryByText('跟随全局')).not.toBeInTheDocument()
    // 英文标签存在（Off + 兜底 low/medium/high）。
    expect(screen.getByText('Off')).toBeInTheDocument()
    expect(screen.getByText('Medium')).toBeInTheDocument()
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

  it('始终思考模型保留旧 Off 的明确提示且不能再次选择 Off', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: ['low', 'high', 'max'], offMode: 'unsupported' })
    const onChange = vi.fn()
    await act(async () => { render(<ThinkingLevelSelector value="off" currentProviderId="p1" currentModel="kimi-k3" onChange={onChange} />) })
    expect(screen.getByRole('button')).toHaveTextContent(/Off.*(不可用|unavailable)/)
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByRole('menuitemradio', { name: 'Off' })).toBeDisabled()
    expect(screen.getByRole('note')).toHaveTextContent(/不支持关闭|cannot turn thinking off/)
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Low' }))
    expect(onChange).toHaveBeenCalledWith('low')
  })

  it('无强度档位的始终思考模型允许修正旧 Off 为 On', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'unsupported' })
    const onChange = vi.fn()
    await act(async () => { render(<ThinkingLevelSelector value="off" currentProviderId="p1" currentModel="kimi-k2.7-code" onChange={onChange} />) })
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByRole('menuitemradio', { name: 'Off' })).toBeDisabled()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'On' }))
    expect(onChange).toHaveBeenCalledWith('high')
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
