import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { GoalCard } from './GoalCard'

describe('GoalCard', () => {
  it('shows a completion timestamp and k tokens with exact usage on hover', () => {
    const finished = new Date('2026-09-08T01:23:00+08:00')
    const { container } = render(<GoalCard
      goal={{ id: 'done', version: 1, objective: '检查商品', status: 'completed', criteria: [],
        total_tokens: 6485801, completed_at: finished.getTime() / 1000, updated_at: finished.getTime() / 1000 + 600 }}
      onEdit={vi.fn()} onPause={vi.fn()} onResume={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText('6485.8k tokens')).toHaveAttribute('title', `${(6485801).toLocaleString()} tokens`)
    expect(container.querySelector('time')).toHaveAttribute('datetime', finished.toISOString())
    expect(container.querySelector('time')).toHaveTextContent('完成于')
  })

  it('defaults to collapsed and routes controls without requiring expansion', async () => {
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
    expect(screen.queryByRole('list', { name: 'Goal 验收清单' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '展开 Goal 详情' })).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(screen.getByRole('button', { name: '展开 Goal 详情' }))
    expect(screen.getByRole('list', { name: 'Goal 验收清单' })).toBeInTheDocument()
    expect(screen.getByText('implemented')).toBeInTheDocument()
    expect(screen.getByText('tested')).toBeInTheDocument()
    expect(screen.getByText('321 tokens')).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText('暂停 Goal'))
    expect(pause).toHaveBeenCalledOnce()
    await waitFor(() => expect(screen.getByLabelText('暂停 Goal')).toBeEnabled())

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
    await waitFor(() => expect(screen.getByLabelText('继续 Goal')).toBeEnabled())
    fireEvent.click(screen.getByLabelText('终止 Goal'))
    expect(cancel).toHaveBeenCalledOnce()
    await waitFor(() => expect(screen.getByLabelText('终止 Goal')).toBeEnabled())
  })

  it('edits multiline text inline and keeps the draft visible on a save error', async () => {
    const onEdit = vi.fn().mockRejectedValueOnce(new Error('保存失败')).mockResolvedValueOnce(undefined)
    render(<GoalCard goal={{ id: 'g', version: 1, objective: '原目标', status: 'paused', criteria: [] }}
      onEdit={onEdit} onPause={vi.fn()} onResume={vi.fn()} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: '编辑 Goal' }))
    const editor = screen.getByRole('textbox', { name: '目标内容' })
    expect(editor).toHaveValue('原目标')
    expect(screen.getByRole('button', { name: '保存目标' })).toBeDisabled()
    fireEvent.change(editor, { target: { value: '新目标\n第二个要求' } })
    fireEvent.click(screen.getByRole('button', { name: '保存目标' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('保存失败')
    expect(editor).toHaveValue('新目标\n第二个要求')
    fireEvent.keyDown(editor, { key: 'Enter', ctrlKey: true })
    await waitFor(() => expect(screen.queryByRole('textbox')).not.toBeInTheDocument())
    expect(onEdit).toHaveBeenLastCalledWith('新目标\n第二个要求')
  })

  it('cancel discards the editor draft and a different goal starts collapsed', () => {
    const props = { onEdit: vi.fn(), onPause: vi.fn(), onResume: vi.fn(), onCancel: vi.fn() }
    const goal = { id: 'g', version: 1, objective: '原目标', status: 'paused' as const, criteria: [] }
    const { rerender } = render(<GoalCard {...props} goal={goal} />)
    fireEvent.click(screen.getByRole('button', { name: '编辑 Goal' }))
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '未保存' } })
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(props.onEdit).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '展开 Goal 详情' }))
    rerender(<GoalCard {...props} goal={{ ...goal, id: 'other' }} />)
    expect(screen.getByRole('button', { name: '展开 Goal 详情' })).toHaveAttribute('aria-expanded', 'false')
  })
})
