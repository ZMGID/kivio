import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ThinkingLevelSelector } from './ThinkingLevelSelector'

// api 在 jsdom 無 Tauri 環境，mock 成確定值；等級清單走兜底也是同樣結果。
vi.mock('../api/tauri', () => ({
  api: {
    getSettings: () => Promise.resolve({ providers: [] }),
    reasoningEffortsForModel: () => Promise.resolve(['low', 'medium', 'high']),
  },
}))

describe('ThinkingLevelSelector', () => {
  it('value=null 時按預設檔顯示 High（不再有「跟隨全域」）', () => {
    render(
      <ThinkingLevelSelector
        value={null}
        currentProviderId="p1"
        currentModel="m1"
        onChange={() => {}}
      />,
    )
    expect(screen.getByRole('button')).toHaveTextContent('High')
  })

  it('下拉項為英文標籤且不含「跟隨全域」', () => {
    render(
      <ThinkingLevelSelector
        value="high"
        currentProviderId="p1"
        currentModel="m1"
        onChange={() => {}}
      />,
    )
    act(() => {
      fireEvent.click(screen.getByRole('button'))
    })
    expect(screen.queryByText('跟隨全域')).not.toBeInTheDocument()
    // 英文標籤存在（Off + 兜底 low/medium/high）。
    expect(screen.getByText('Off')).toBeInTheDocument()
    expect(screen.getByText('Medium')).toBeInTheDocument()
  })

  it('選擇某一檔回撥原始等級值', () => {
    const onChange = vi.fn()
    render(
      <ThinkingLevelSelector
        value="high"
        currentProviderId="p1"
        currentModel="m1"
        onChange={onChange}
      />,
    )
    act(() => {
      fireEvent.click(screen.getByRole('button'))
    })
    act(() => {
      fireEvent.click(screen.getByText('Off'))
    })
    expect(onChange).toHaveBeenCalledWith('off')
  })
})
