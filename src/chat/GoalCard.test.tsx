import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { GoalCard } from './GoalCard'

describe('GoalCard', () => {
  it('shows verified criteria and routes pause/resume/cancel actions by status', () => {
    const pause = vi.fn()
    const cancel = vi.fn()
    const { rerender } = render(
      <GoalCard
        goal={{
          id: 'g1', version: 1, objective: 'Ship Goal mode', status: 'active',
          criteria: [
            { id: 'c1', text: 'implemented', verified: true },
            { id: 'c2', text: 'tested', verified: false },
          ],
          total_tokens: 321,
        }}
        onEdit={vi.fn()}
        onPause={pause}
        onResume={vi.fn()}
        onCancel={cancel}
      />,
    )
    expect(screen.getByText('已验证 1/2 项')).toBeInTheDocument()
    expect(screen.getByRole('list', { name: 'Goal 验收清单' })).toBeInTheDocument()
    expect(screen.getByText('implemented')).toBeInTheDocument()
    expect(screen.getByText('tested')).toBeInTheDocument()
    expect(screen.getByText('321 tokens')).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText('暂停 Goal'))
    expect(pause).toHaveBeenCalledOnce()

    const resume = vi.fn()
    rerender(
      <GoalCard
        goal={{ id: 'g1', version: 2, objective: 'Ship Goal mode', status: 'paused', criteria: [] }}
        onEdit={vi.fn()}
        onPause={pause}
        onResume={resume}
        onCancel={cancel}
      />,
    )
    fireEvent.click(screen.getByLabelText('继续 Goal'))
    expect(resume).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByLabelText('终止 Goal'))
    expect(cancel).toHaveBeenCalledOnce()
  })
})
