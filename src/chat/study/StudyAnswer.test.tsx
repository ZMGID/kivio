import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { StudyAnswer } from './StudyAnswer'
vi.mock('../../components/i18n', () => ({ useLang: () => 'en' }))
vi.mock('../ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <p>{content}</p> }))
afterEach(cleanup)

describe('Study direct model replies', () => {
  it.each(['hint', 'check', 'explain'] as const)('streams ordinary %s replies without a structured-response protocol or extra reveal step', mode => {
    const view = render(<StudyAnswer turn={{ mode, answer: 'First streamed step', status: 'streaming' }} />)
    expect(screen.getByText('First streamed step')).toBeVisible()
    view.rerender(<StudyAnswer turn={{ mode, answer: 'First streamed step, then a question.', status: 'complete' }} />)
    expect(screen.getByText('First streamed step, then a question.')).toBeVisible()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })
  it.each([[false, 'complete'], [true, 'complete'], [false, 'interrupted'], [true, 'cancelled'], [false, 'error']] as const)('reads earlier saved structured replies without exposing their withheld solution (fenced=%s, status=%s)', (fenced, status) => {
    const answer = JSON.stringify({ version: 1, mode: 'check', firstIssue: 'Check the factor.', nextStep: 'Rewrite the differential.', verification: 'Your derivative differs.', withheldSolution: 'OLD HIDDEN ANSWER' })
    render(<StudyAnswer turn={{ mode: 'check', answer: fenced ? '```json\n' + answer + '\n```' : answer, status }} />)
    expect(screen.getByText(/Check the factor/)).toBeVisible()
    expect(screen.queryByText(/OLD HIDDEN ANSWER/)).not.toBeInTheDocument()
    expect(screen.queryByText(/firstIssue/)).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal previously saved solution' }))
    expect(screen.getByText('OLD HIDDEN ANSWER')).toBeVisible()
  })
  it('keeps interrupted partial replies readable', () => {
    render(<StudyAnswer turn={{ mode: 'hint', answer: 'Partial hint', status: 'interrupted' }} />)
    expect(screen.getByText('Partial hint')).toBeVisible()
  })
  it('preserves explicit full-solution reveal and re-hiding', () => {
    render(<StudyAnswer turn={{ mode: 'solution', answer: 'full solution', status: 'complete' }} />)
    expect(screen.queryByText('full solution')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal full solution' }))
    expect(screen.getByText('full solution')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Hide response' }))
    expect(screen.queryByText('full solution')).not.toBeInTheDocument()
  })
})
