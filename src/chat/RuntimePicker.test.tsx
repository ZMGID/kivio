import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ExternalModelSelector } from './RuntimePicker'
import type { AgentRuntimeConfig } from './types'

const detectModels = vi.fn()

vi.mock('./api', () => ({
  chatApi: {
    detectExternalAgents: vi.fn(() => Promise.resolve([])),
    detectExternalAgentModels: (...args: unknown[]) => detectModels(...args),
  },
}))

const runtime: AgentRuntimeConfig = {
  kind: 'external',
  externalAgentId: 'cursor',
  externalModel: 'default',
  externalReasoning: null,
}

describe('ExternalModelSelector', () => {
  beforeEach(() => {
    detectModels.mockReset()
  })

  it('探测失败降级时展示默认列表提示，重试以 force=true 重探', async () => {
    detectModels
      .mockResolvedValueOnce({
        models: [{ id: 'default', label: 'Default' }],
        reasoningOptions: [],
        source: 'fallback',
        probeError: 'boom',
      })
      .mockResolvedValueOnce({
        models: [{ id: 'gpt-5', label: 'gpt-5' }],
        reasoningOptions: [],
        source: 'probed',
      })

    render(
      <ExternalModelSelector
        agentRuntime={runtime}
        onModelChange={() => {}}
        conversationId={null}
      />,
    )

    // 初次懒查（fallback），force 未传。
    await waitFor(() => expect(detectModels).toHaveBeenCalledTimes(1))
    expect(detectModels.mock.calls[0][2]).toBeUndefined()

    // 打开下拉：出现降级提示行。
    act(() => {
      fireEvent.click(screen.getByRole('button'))
    })
    expect(screen.getByText('探测失败，显示默认列表')).toBeInTheDocument()

    // 点重试 → 以 force=true 重探。
    act(() => {
      fireEvent.click(screen.getByText('重试'))
    })
    await waitFor(() => expect(detectModels).toHaveBeenCalledTimes(2))
    expect(detectModels.mock.calls[1][2]).toBe(true)
  })

  it('probed 结果不显示降级提示', async () => {
    detectModels.mockResolvedValue({
      models: [{ id: 'gpt-5', label: 'gpt-5' }],
      reasoningOptions: [],
      source: 'probed',
    })

    render(
      <ExternalModelSelector
        agentRuntime={runtime}
        onModelChange={() => {}}
        conversationId={null}
      />,
    )
    await waitFor(() => expect(detectModels).toHaveBeenCalledTimes(1))
    act(() => {
      fireEvent.click(screen.getByRole('button'))
    })
    expect(screen.queryByText('探测失败，显示默认列表')).not.toBeInTheDocument()
    expect(screen.getByText('gpt-5')).toBeInTheDocument()
  })
})
