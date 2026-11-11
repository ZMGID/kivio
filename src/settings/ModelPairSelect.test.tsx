import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ModelPairSelect } from './ModelPairSelect'
import { makeProvider } from './tabs/testFixtures'

describe('ModelPairSelect', () => {
  it('shows a disabled model prompt when no provider is configured', () => {
    render(<ModelPairSelect providerId="" model="" providers={[]} onChange={vi.fn()} />)
    expect(screen.getByRole('button', { name: '选择模型' })).toBeDisabled()
    expect(screen.queryByText('gpt-4o')).not.toBeInTheDocument()
    expect(screen.queryByText('["",""]')).not.toBeInTheDocument()
  })

  it('keeps an explicit model selection', () => {
    render(<ModelPairSelect providerId="p1" model="gpt-4o" providers={[makeProvider()]} onChange={vi.fn()} />)
    expect(screen.getByRole('button', { name: /gpt-4o/ })).toBeEnabled()
  })

  it('lets the user choose a configured model from an empty selection', async () => {
    const onChange = vi.fn()
    render(<ModelPairSelect providerId="" model="" providers={[makeProvider()]} onChange={onChange} />)
    const trigger = screen.getByRole('button', { name: '选择模型' })
    expect(trigger).toBeEnabled()
    expect(onChange).not.toHaveBeenCalled()
    await userEvent.click(trigger)
    await userEvent.click(screen.getByRole('option', { name: /gpt-4o/ }))
    expect(onChange).toHaveBeenCalledWith('p1', 'gpt-4o')
  })

  it('preserves the inherit option without configured providers', () => {
    render(<ModelPairSelect providerId="" model="" providers={[]} inheritLabel="跟随当前模型" onChange={vi.fn()} />)
    expect(screen.getByRole('button', { name: '跟随当前模型' })).toBeEnabled()
  })
})
