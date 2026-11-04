import { act, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type ConversationCost } from '../api/tauri'
import { SessionUsageStrip } from './SessionUsageStrip'
import type { ChatMessage, MessageUsage } from './types'
import { useSubAgents } from './useSubAgents'

vi.mock('../api/tauri', () => ({ api: { usageGetConversationCost: vi.fn() } }))
vi.mock('./useSubAgents', () => ({ useSubAgents: vi.fn(() => ({ agents: [], error: '' })) }))

let seq = 0

function assistant(usage: MessageUsage, providerId?: string): ChatMessage {
  seq += 1
  return {
    id: `m-${seq}`,
    role: 'assistant',
    content: 'x',
    timestamp: 1,
    usage,
    provider_id: providerId,
  }
}

describe('SessionUsageStrip', () => {
  beforeEach(() => {
    vi.mocked(api.usageGetConversationCost).mockReset()
    vi.mocked(useSubAgents).mockReturnValue({ agents: [], error: '' })
  })

  it('refreshes cost when a child finishes without a parent message update', async () => {
    const child = { id: 'child', name: 'Worker', sequence: 1,
      profile: { model: 'test', agentType: 'researcher' },
      runs: [{ id: 'run', status: 'running', prompt: '' }], messages: [], history: [], tools: [] }
    vi.mocked(useSubAgents).mockReturnValue({ agents: [child], error: '' })
    vi.mocked(api.usageGetConversationCost)
      .mockResolvedValueOnce({ costUsd: 0.25, unpricedRequests: 0, skippedRecords: 0 })
      .mockResolvedValueOnce({ costUsd: 0.75, unpricedRequests: 0, skippedRecords: 0 })
    const { rerender } = render(<SessionUsageStrip conversationId="parent" lang="zh" messages={[]} />)
    expect(await screen.findByText('0.25$')).toBeTruthy()
    vi.mocked(useSubAgents).mockReturnValue({ agents: [{ ...child, preview: 'working' }], error: '' })
    rerender(<SessionUsageStrip conversationId="parent" lang="zh" messages={[]} />)
    expect(api.usageGetConversationCost).toHaveBeenCalledTimes(1)
    vi.mocked(useSubAgents).mockReturnValue({ agents: [{ ...child, runs: [{ ...child.runs[0], status: 'returned' }] }], error: '' })
    rerender(<SessionUsageStrip conversationId="parent" lang="zh" messages={[]} />)
    expect(await screen.findByText('0.75$')).toBeTruthy()
  })

  it('shows every metric immediately without a disclosure control', async () => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce({ costUsd: 0.25, unpricedRequests: 0, skippedRecords: 0 })
    const { container } = render(<SessionUsageStrip conversationId="parent" lang="en"
      messages={[assistant({ input_tokens: 100, cached_input_tokens: 80, output_tokens: 20 })]} />)
    expect(await screen.findByText('0.25$')).toBeVisible()
    expect(screen.getByText('80%')).toBeVisible()
    expect(screen.getByLabelText('Input')).toHaveTextContent('↑20')
    expect(screen.getByLabelText('Output')).toHaveTextContent('↓20')
    expect(container.querySelector('details, summary, button')).toBeNull()
  })

  it.each([
    [0.0234, 0, '0.02$'],
    [0, 0, '0.00$'],
    [0.00001, 0, '0.00$'],
    [1.25, 2, '1.25$'],
    [null, 1, '—$'],
  ])('displays recorded cost %s with %s unpriced requests as %s', async (costUsd, unpricedRequests, label) => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce({ costUsd, unpricedRequests, skippedRecords: 0 })
    render(<SessionUsageStrip conversationId="cost-chat" lang="zh" messages={[]} />)
    await act(async () => {})
    expect(screen.getByText(label)).toBeTruthy()
    expect(screen.queryByRole('heading', { name: '已加载消息用量' })).not.toBeInTheDocument()
  })

  it.each([
    [{ costUsd: 1.25, unpricedRequests: 2, skippedRecords: 0 }, '1.25$', true],
    [{ costUsd: 0.5, unpricedRequests: 0, skippedRecords: 1 }, '0.50$', true],
    [{ costUsd: null, unpricedRequests: 1, skippedRecords: 0 }, '—$', false],
  ])('distinguishes partial or unknown cost from a complete invoice', async (cost, value, hasKnownCost) => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce(cost)
    render(<SessionUsageStrip conversationId="cost-chat" lang="en" messages={[]} />)
    await act(async () => {})
    expect(screen.getByText(value)).toBeVisible()
    expect(screen.getByText(value).parentElement!).toHaveTextContent('*')
    const card = screen.getByRole('tooltip', { hidden: true })
    expect(card).toHaveTextContent('not an invoice')
    expect(card).toHaveTextContent('Some requests lack pricing or usage')
    expect(card.textContent!.includes('Only known costs are shown')).toBe(hasKnownCost)
    if (!hasKnownCost) expect(screen.queryByText('0.00$')).not.toBeInTheDocument()
  })

  it('isolates late costs across navigation and refreshes after generation ends', async () => {
    let finishOld!: (value: ConversationCost) => void
    vi.mocked(api.usageGetConversationCost)
      .mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve }))
      .mockResolvedValueOnce({ costUsd: 0.25, unpricedRequests: 0, skippedRecords: 0 })
      .mockResolvedValueOnce({ costUsd: 0.5, unpricedRequests: 0, skippedRecords: 0 })
    const { rerender } = render(<SessionUsageStrip conversationId="old" generating lang="zh" messages={[]} />)
    rerender(<SessionUsageStrip conversationId="new" generating lang="zh" messages={[]} />)
    expect(await screen.findByText('0.25$')).toBeTruthy()
    await act(async () => { finishOld({ costUsd: 99, unpricedRequests: 0, skippedRecords: 0 }) })
    expect(screen.queryByText('99.00$')).toBeNull()
    rerender(<SessionUsageStrip conversationId="new" generating={false} lang="zh" messages={[]} />)
    expect(await screen.findByText('0.50$')).toBeTruthy()
  })

  it('includes Anthropic cache writes in fresh input and cache hit denominator', () => {
    render(<SessionUsageStrip lang="zh" defaultApiFormat="anthropic_messages" messages={[
      assistant({ input_tokens: 2, cached_input_tokens: 700, cache_creation_input_tokens: 300 }),
    ]} />)
    expect(screen.getByText('70%')).toBeTruthy()
    expect(screen.getByText('302')).toBeTruthy()
  })

  it('shows loaded input/output and cache hit percentage (Anthropic input excludes cache)', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        apiFormats={{ anthropic: 'anthropic_messages' }}
        defaultApiFormat="anthropic_messages"
        messages={[
          assistant({ input_tokens: 1000, output_tokens: 200, cached_input_tokens: 800 }, 'anthropic'),
          assistant({ input_tokens: 500, output_tokens: 50, cached_input_tokens: 300 }, 'anthropic'),
        ]}
      />,
    )
    // 输入 = 1000+500（不减）；缓存命中率 = 1100 / (1500+1100) ≈ 42.3% → 取整 42%
    expect(screen.getByText('1.5K')).toBeTruthy()
    expect(screen.getByText('42%')).toBeTruthy()
    expect(screen.getByText('250')).toBeTruthy()
  })

  it('subtracts cached tokens from OpenAI-style input and rounds big percentages', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        messages={[assistant({ input_tokens: 10_000, output_tokens: 300, cached_input_tokens: 4_000 })]}
      />,
    )
    expect(screen.getByText('6.0K')).toBeTruthy()
    expect(screen.getByText('40%')).toBeTruthy()
    expect(screen.getByText('300')).toBeTruthy()
  })

  it('resolves per-message provider format over the default', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        defaultApiFormat="openai_chat"
        messages={[
          assistant({ input_tokens: 10_000, cached_input_tokens: 3_000 }, 'anthropic-p'),
          assistant({ input_tokens: 2_000, cached_input_tokens: 500 }),
        ]}
        apiFormats={{ 'anthropic-p': 'anthropic_messages' }}
      />,
    )
    // anthropic-p 不减（10000）；默认 openai_chat 的减（2000−500=1500）
    expect(screen.getByText('11.5K')).toBeTruthy()
    // 3500 / (11500 + 3500) = 23.3% → 取整 23%
    expect(screen.getByText('23%')).toBeTruthy()
  })

  it('does not subtract cache from dsh-style input (already exclusive)', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        defaultApiFormat="openai_chat"
        cacheIncludedInInput={false}
        messages={[
          assistant({ input_tokens: 457, output_tokens: 1442, cached_input_tokens: 51_456 }),
          assistant({ input_tokens: 122, output_tokens: 600, cached_input_tokens: 54_528 }),
        ]}
      />,
    )
    // 再减一次会变成 ↑0 / 缓存 100%。新鲜输入 457+122=579。
    expect(screen.getByText('579')).toBeTruthy()
    expect(screen.getByText('99%')).toBeTruthy()
    expect(screen.getByText('2.0K')).toBeTruthy()
  })

  it('supports camelCase fields and a reported zero cache hit rate', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        messages={[assistant({ inputTokens: 200, outputTokens: 40, cachedInputTokens: 0 })]}
      />,
    )
    expect(screen.getByText('200')).toBeTruthy()
    expect(screen.getByText('40')).toBeTruthy()
    expect(screen.getByText('0%')).toBeTruthy()
  })

  it('shows unknown fields as unknown instead of zero', () => {
    render(<SessionUsageStrip lang="en" messages={[assistant({ input_tokens: 200 })]} />)
    expect(screen.getByLabelText('Input')).toHaveTextContent('↑200')
    expect(screen.getByLabelText('Output')).toHaveTextContent('↓—')
    expect(screen.getByLabelText('Cache hit')).toHaveTextContent('—')
  })

  it.each([
    [[0, 0, 0], [0, 0, 0]],
    [[1, 1, 2], [6, 6, 12]],
    [[1_000_000, 1, 0], [23, 1, 0]],
    [[1, 1, 1], [8, 8, 8]],
  ])('renders usage %j as %j pixels', ([cached, input, output], expected) => {
    const { container } = render(<SessionUsageStrip lang="en" cacheIncludedInInput={false}
      messages={[assistant({ cached_input_tokens: cached, input_tokens: input, output_tokens: output })]} />)
    const grid = container.querySelector('.chat-session-usage-grid')
    expect(grid?.children).toHaveLength(24)
    expect(['cache', 'input', 'output'].map(kind => grid?.querySelectorAll(`[data-k="${kind}"]`).length))
      .toEqual(expected)
  })

  it('renders nothing when no message carries usage', () => {
    const { container } = render(
      <SessionUsageStrip
        lang="zh"
        messages={[
          assistant({}),
          { id: 'u-1', role: 'user', content: 'hi', timestamp: 1 },
        ]}
      />,
    )
    expect(container.firstChild).toBeNull()
  })

  it('renders nothing for an empty conversation', () => {
    const { container } = render(<SessionUsageStrip lang="zh" messages={[]} />)
    expect(container.firstChild).toBeNull()
  })
})
