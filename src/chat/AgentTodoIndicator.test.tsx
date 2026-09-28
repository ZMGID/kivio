import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { LangContext } from '../components/i18n'
import { AgentTodoIndicator } from './AgentTodoIndicator'
import type { AgentTodoState } from './types'

const state: AgentTodoState = {
  items: [
    { id: '1', content: 'Read the protocol', status: 'completed' },
    { id: '2', content: 'Old approach', status: 'cancelled' },
    { id: '3', content: 'Wire it up', status: 'in_progress', blocked_by: ['1'] },
  ],
  updated_at: 1,
}

describe('AgentTodoIndicator', () => {
  it('leaves cancelled items out of the count', () => {
    render(<AgentTodoIndicator todoState={state} placement="status" />)
    expect(screen.getByRole('button', { name: 'Agent 待办' })).toHaveTextContent('待办 1/2')
  })

  it('shows the finished chip once nothing is pending', () => {
    const done: AgentTodoState = {
      items: [
        { id: '1', content: 'A', status: 'completed' },
        { id: '2', content: 'B', status: 'cancelled' },
      ],
      updated_at: 1,
    }
    render(<AgentTodoIndicator todoState={done} placement="status" />)
    expect(screen.getByRole('button', { name: 'Agent 待办' })).toHaveTextContent('完成 1/1')
  })

  it('localizes the panel and names blockers by content', () => {
    render(
      <LangContext.Provider value="en">
        <AgentTodoIndicator todoState={state} />
      </LangContext.Provider>,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Agent todo' }))
    expect(screen.getByText('Now')).toBeInTheDocument()
    expect(screen.getByText('Skipped')).toBeInTheDocument()
    expect(screen.getByText('Waiting on: Read the protocol')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Close todo panel' })).toBeInTheDocument()
  })

  it('renders nothing for an empty list', () => {
    const { container } = render(<AgentTodoIndicator todoState={{ items: [], updated_at: 0 }} />)
    expect(container).toBeEmptyDOMElement()
  })
})
