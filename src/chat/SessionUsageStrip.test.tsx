import { act, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type ConversationCost } from '../api/tauri'
import { SessionUsageStrip } from './SessionUsageStrip'
import type { ChatMessage, MessageUsage } from './types'

vi.mock('../api/tauri', () => ({ api: { usageGetConversationCost: vi.fn() } }))

let seq = 0

/** 悬停信息卡：数字的权威展示处；底栏常驻行是同一份数据的 aria-hidden 摘要。 */
const card = () => within(screen.getByRole('tooltip', { hidden: true }))

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
  })

  it('refreshes the backend cost when a result is added, but not on text updates', async () => {
    vi.mocked(api.usageGetConversationCost)
      .mockResolvedValueOnce({ costUsd: 0.25, unpricedRequests: 0, skippedRecords: 0 })
      .mockResolvedValueOnce({ costUsd: 0.75, unpricedRequests: 0, skippedRecords: 0 })
    const message: ChatMessage = { id: 'parent-message', role: 'assistant', content: 'Working', timestamp: 1 }
    const { rerender } = render(<SessionUsageStrip conversationId="parent" lang="zh" messages={[message]} />)
    expect(await card().findByText('$0.25')).toBeTruthy()
    rerender(<SessionUsageStrip conversationId="parent" lang="zh" messages={[{ ...message, content: 'Still working' }]} />)
    expect(api.usageGetConversationCost).toHaveBeenCalledTimes(1)
    rerender(<SessionUsageStrip conversationId="parent" lang="zh" messages={[
      { id: 'subagent-result-job', role: 'assistant', content: 'Done', timestamp: 2 }, message,
    ]} />)
    expect(await card().findByText('$0.75')).toBeTruthy()
    expect(api.usageGetConversationCost).toHaveBeenLastCalledWith('parent')
  })

  it('shows every metric immediately without a disclosure control', async () => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce({ costUsd: 0.25, unpricedRequests: 0, skippedRecords: 0 })
    const { container } = render(<SessionUsageStrip conversationId="parent" lang="en"
      messages={[assistant({ input_tokens: 100, cached_input_tokens: 80, output_tokens: 20 })]} />)
    expect(await card().findByText('$0.25')).toBeTruthy()
    expect(container.querySelector('.chat-session-usage-readout')).toHaveTextContent('$0.25')
    expect(container.querySelectorAll('.chat-session-usage-spectrum > i')).toHaveLength(3)
    expect(card().getByLabelText('Input')).toHaveTextContent('↑20')
    expect(card().getByLabelText('Output')).toHaveTextContent('↓20')
    expect(container.querySelector('details, summary, button')).toBeNull()
  })

  it.each([
    [0.0234, 0, '$0.02'],
    [0, 0, '$0.00'],
    [0.00001, 0, '$0.00'],
    [1.25, 2, '$1.25'],
    [null, 1, '$—'],
  ])('displays recorded cost %s with %s unpriced requests as %s', async (costUsd, unpricedRequests, label) => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce({ costUsd, unpricedRequests, skippedRecords: 0 })
    render(<SessionUsageStrip conversationId="cost-chat" lang="zh" messages={[]} />)
    await act(async () => {})
    expect(card().getByText(label)).toBeTruthy()
    expect(screen.queryByRole('heading', { name: '已加载消息用量' })).not.toBeInTheDocument()
  })

  it.each([
    [{ costUsd: 1.25, unpricedRequests: 2, skippedRecords: 0 }, '$1.25', true],
    [{ costUsd: 0.5, unpricedRequests: 0, skippedRecords: 1 }, '$0.50', true],
    [{ costUsd: null, unpricedRequests: 1, skippedRecords: 0 }, '$—', false],
  ])('distinguishes partial or unknown cost from a complete invoice', async (cost, value, hasKnownCost) => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce(cost)
    render(<SessionUsageStrip conversationId="cost-chat" lang="en" messages={[]} />)
    await act(async () => {})
    expect(card().getByText(value)).toBeVisible()
    expect(card().getByText(value).parentElement!).toHaveTextContent('*')
    const tooltip = screen.getByRole('tooltip', { hidden: true })
    expect(tooltip).toHaveTextContent('not an invoice')
    expect(tooltip).toHaveTextContent('Some requests lack pricing or usage')
    expect(tooltip.textContent!.includes('Only known costs are shown')).toBe(hasKnownCost)
    if (!hasKnownCost) expect(card().queryByText('$0.00')).not.toBeInTheDocument()
  })

  it('isolates late costs across navigation and refreshes after generation ends', async () => {
    let finishOld!: (value: ConversationCost) => void
    vi.mocked(api.usageGetConversationCost)
      .mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve }))
      .mockResolvedValueOnce({ costUsd: 0.25, unpricedRequests: 0, skippedRecords: 0 })
      .mockResolvedValueOnce({ costUsd: 0.5, unpricedRequests: 0, skippedRecords: 0 })
    const { rerender } = render(<SessionUsageStrip conversationId="old" generating lang="zh" messages={[]} />)
    rerender(<SessionUsageStrip conversationId="new" generating lang="zh" messages={[]} />)
    expect(await card().findByText('$0.25')).toBeTruthy()
    await act(async () => { finishOld({ costUsd: 99, unpricedRequests: 0, skippedRecords: 0 }) })
    expect(card().queryByText('$99.00')).toBeNull()
    rerender(<SessionUsageStrip conversationId="new" generating={false} lang="zh" messages={[]} />)
    expect(await card().findByText('$0.50')).toBeTruthy()
  })

  it('includes Anthropic cache writes in fresh input and cache hit denominator', () => {
    render(<SessionUsageStrip lang="zh" defaultApiFormat="anthropic_messages" messages={[
      assistant({ input_tokens: 2, cached_input_tokens: 700, cache_creation_input_tokens: 300 }),
    ]} />)
    expect(card().getByText('70%')).toBeTruthy()
    expect(card().getByText('302')).toBeTruthy()
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
    expect(card().getByText('1.5K')).toBeTruthy()
    expect(card().getByText('42%')).toBeTruthy()
    expect(card().getByText('250')).toBeTruthy()
  })

  it('subtracts cached tokens from OpenAI-style input and rounds big percentages', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        messages={[assistant({ input_tokens: 10_000, output_tokens: 300, cached_input_tokens: 4_000 })]}
      />,
    )
    expect(card().getByText('6.0K')).toBeTruthy()
    expect(card().getByText('40%')).toBeTruthy()
    expect(card().getByText('300')).toBeTruthy()
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
    expect(card().getByText('11.5K')).toBeTruthy()
    // 3500 / (11500 + 3500) = 23.3% → 取整 23%
    expect(card().getByText('23%')).toBeTruthy()
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
    expect(card().getByText('579')).toBeTruthy()
    expect(card().getByText('99%')).toBeTruthy()
    expect(card().getByText('2.0K')).toBeTruthy()
  })

  it('supports camelCase fields and a reported zero cache hit rate', () => {
    render(
      <SessionUsageStrip
        lang="zh"
        messages={[assistant({ inputTokens: 200, outputTokens: 40, cachedInputTokens: 0 })]}
      />,
    )
    expect(card().getByText('200')).toBeTruthy()
    expect(card().getByText('40')).toBeTruthy()
    expect(card().getByText('0%')).toBeTruthy()
  })

  it('shows unknown fields as unknown instead of zero', () => {
    render(<SessionUsageStrip lang="en" messages={[assistant({ input_tokens: 200 })]} />)
    expect(card().getByLabelText('Input')).toHaveTextContent('↑200')
    expect(card().getByLabelText('Output')).toHaveTextContent('↓—')
    expect(card().getByLabelText('Cache hit')).toHaveTextContent('—')
  })

  it.each([
    [{ costUsd: 1.25, unpricedRequests: 2, skippedRecords: 0 }, [], '$1.25*'],
    [{ costUsd: null, unpricedRequests: 0, skippedRecords: 0 }, [], '$—'],
    [{ costUsd: 0.5, unpricedRequests: 0, skippedRecords: 0 }, [assistant({ input_tokens: 200 })], '$0.50'],
  ])('keeps a text readout in the composer bar for cost %j', async (cost, messages, expected) => {
    vi.mocked(api.usageGetConversationCost).mockResolvedValueOnce(cost)
    const { container } = render(<SessionUsageStrip conversationId="c" lang="en" messages={messages} />)
    await act(async () => {})
    expect(container.querySelector('.chat-session-usage-readout')!.textContent).toBe(expected)
  })

  it('omits cost for external CLI sessions and renders nothing until usage arrives', () => {
    const { container, rerender } = render(<SessionUsageStrip conversationId="c" hideCost lang="en" messages={[]} />)
    expect(container.firstChild).toBeNull()
    rerender(<SessionUsageStrip conversationId="c" hideCost lang="en" cacheIncludedInInput={false}
      messages={[assistant({ input_tokens: 20, cached_input_tokens: 80, output_tokens: 5 })]} />)
    expect(container.querySelector('.chat-session-usage-readout')!.textContent).toBe('↑20 ↓5')
    expect([...container.querySelectorAll<HTMLElement>('.chat-session-usage-spectrum > i')].map(i => [i.dataset.k, i.style.flexGrow]))
      .toEqual([['cache', '80'], ['input', '20'], ['output', '5']])
    expect(api.usageGetConversationCost).not.toHaveBeenCalled()
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
