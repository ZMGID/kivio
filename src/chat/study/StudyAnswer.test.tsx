import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { StudyAnswer } from './StudyAnswer'
import type { StudyTurn } from './studyStorage'
vi.mock('../../components/i18n', () => ({ useLang: () => 'en' }))
vi.mock('../ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <p>{content}</p> }))
afterEach(cleanup)
const turn = (answer: string, extra: Partial<StudyTurn> = {}) => ({ mode: 'check' as const, answer, status: 'complete' as const, ...extra })
const structured = JSON.stringify({ version: 1, mode: 'check', verification: 'Differentiating your result gives three times the integrand.', firstIssue: 'Line 3 loses a factor.', nextStep: 'Rewrite the differential.', withheldSolution: 'SPOILER: arctan(x^3)/3+C' })

describe('Study answer disclosure', () => {
  it('shows the first issue but mounts verification and optional solutions only on explicit reveal', () => {
    render(<StudyAnswer turn={turn(structured)} />)
    expect(screen.getByText('Line 3 loses a factor.')).toBeVisible()
    expect(screen.queryByText(/SPOILER/)).not.toBeInTheDocument()
    expect(screen.queryByText('Differentiating your result gives three times the integrand.')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'View verification' }))
    expect(screen.getByText('Differentiating your result gives three times the integrand.')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal extra explanation (may include the answer)' }))
    expect(screen.getByText(/SPOILER/)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Hide response' }))
    expect(screen.queryByText(/SPOILER/)).not.toBeInTheDocument()
  })
  it.each(['hint', 'check'] as const)('never mounts raw %s tokens while streaming or after malformed completion', mode => {
    const view = render(<StudyAnswer turn={turn('SPOILER: full worked answer', { mode, status: 'streaming' })} />)
    expect(screen.queryByText(/SPOILER/)).not.toBeInTheDocument()
    view.rerender(<StudyAnswer turn={turn('SPOILER: full worked answer', { mode })} />)
    expect(screen.queryByText(/SPOILER/)).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'View original reply (may reveal the answer)' }))
    expect(screen.getByText('SPOILER: full worked answer')).toBeVisible()
  })
  it('keeps interrupted partial replies available without showing them automatically', () => {
    render(<StudyAnswer turn={turn('partial full answer', { status: 'interrupted' })} />)
    expect(screen.queryByText('partial full answer')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'View original reply (may reveal the answer)' }))
    expect(screen.getByText('partial full answer')).toBeVisible()
  })
  it('preserves explicit full-solution reveal and normal explanation rendering', () => {
    const view = render(<StudyAnswer turn={turn('full solution', { mode: 'solution' })} />)
    expect(screen.queryByText('full solution')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal full solution' }))
    expect(screen.getByText('full solution')).toBeVisible()
    view.rerender(<StudyAnswer turn={turn('concept explanation', { mode: 'explain' })} />)
    expect(screen.getByText('concept explanation')).toBeVisible()
  })
})
