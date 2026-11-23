import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { DegradedAnswerCard } from './DegradedAnswerCard'
import type { DegradedAnswer } from './types'

function makeDegraded(overrides: Partial<DegradedAnswer> = {}): DegradedAnswer {
  return {
    kind: 'rate_limited',
    reason: '模型调用被限流或配额耗尽，多次退避重试后仍失败。',
    text: '⚠️ ...纯文本版本...',
    ...overrides,
  }
}

describe('DegradedAnswerCard', () => {
  it('shows the failure without repeating tool activity from older replies', () => {
    const oldReply = {
      ...makeDegraded(),
      totalToolCalls: 9,
      toolSummaries: [{ name: 'bash', preview: 'exit_code: 0' }],
    }
    render(<DegradedAnswerCard degraded={oldReply} />)
    expect(screen.getByText(/限流或配额耗尽/)).toBeVisible()
    expect(screen.queryByText(/工具调用|工具结果|exit_code/)).toBeNull()
  })

  it('有 detail 时显示供应商原始报错', () => {
    render(<DegradedAnswerCard degraded={makeDegraded({ detail: 'context deadline exceeded' })} />)
    expect(screen.getByText('context deadline exceeded')).toBeTruthy()
  })

  it('按 kind 选择标签（不解析文案）', () => {
    const cases: Array<[string, string]> = [
      ['rate_limited', '限流 / 配额'],
      ['context_overflow', '上下文超长'],
      ['timeout', '超时 / 连接中断'],
      ['moderation', '内容审核拦截'],
      ['empty_response', '空响应'],
      ['invalid_image', '图片无效'],
      ['invalid_request', '请求被拒绝'],
    ]
    for (const [kind, label] of cases) {
      const { unmount } = render(<DegradedAnswerCard degraded={makeDegraded({ kind })} />)
      expect(screen.getByText(label)).toBeTruthy()
      unmount()
    }
  })

  it('does not render obsolete collaboration verdicts from history', () => {
    const { container } = render(<DegradedAnswerCard degraded={makeDegraded({ kind: 'collaboration_incomplete' })} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('未知 kind 回落到通用标签而非崩溃', () => {
    render(<DegradedAnswerCard degraded={makeDegraded({ kind: 'something_new' })} />)
    expect(screen.getByText('调用失败')).toBeTruthy()
  })

  it('kind 暴露到 DOM 供样式/排查定位', () => {
    const { container } = render(<DegradedAnswerCard degraded={makeDegraded()} />)
    expect(container.querySelector('[data-degraded-kind="rate_limited"]')).toBeTruthy()
  })
})
