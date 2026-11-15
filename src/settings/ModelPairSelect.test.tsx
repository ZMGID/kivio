import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ModelEffortSelect, ModelPairSelect } from './ModelPairSelect'
import { api, type ThinkingCapabilities } from '../api/tauri'
import { makeProvider } from './tabs/testFixtures'

vi.mock('../api/tauri', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/tauri')>(),
  api: { thinkingCapabilitiesForModel: vi.fn() },
}))

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

describe('ModelEffortSelect', () => {
  const providers = [makeProvider()]

  it('offers only supported efforts and saves the selected value', async () => {
    vi.mocked(api.thinkingCapabilitiesForModel).mockResolvedValue({ levels: ['low', 'high'], offMode: 'unsupported' })
    const onChange = vi.fn()
    render(<ModelEffortSelect providerId="p1" model="gpt-5.5" providers={providers} value="" onChange={onChange} />)
    await userEvent.click(await screen.findByRole('button', { name: '推理强度' }))
    expect(screen.queryByRole('option', { name: 'Off' })).toBeNull()
    expect(screen.queryByRole('option', { name: 'medium' })).toBeNull()
    await userEvent.click(screen.getByRole('option', { name: 'high' }))
    expect(onChange).toHaveBeenCalledWith('high')
    expect(api.thinkingCapabilitiesForModel).toHaveBeenCalledWith('gpt-5.5', 'p1')
  })

  it('can turn reasoning off or restore the model default', async () => {
    vi.mocked(api.thinkingCapabilitiesForModel).mockResolvedValue({ levels: [], offMode: 'supported' })
    const onChange = vi.fn()
    render(<ModelEffortSelect providerId="p1" model="toggle-model" providers={providers} value="off" onChange={onChange} />)
    const trigger = await screen.findByRole('button', { name: '推理强度' })
    await userEvent.click(trigger)
    await userEvent.click(screen.getByRole('option', { name: '使用默认' }))
    expect(onChange).toHaveBeenLastCalledWith('')
    await userEvent.click(trigger)
    await userEvent.click(screen.getByRole('option', { name: 'Off' }))
    expect(onChange).toHaveBeenLastCalledWith('off')
  })

  it('hides the control for models with no adjustable reasoning', async () => {
    vi.mocked(api.thinkingCapabilitiesForModel).mockResolvedValue({ levels: [], offMode: 'not_applicable' })
    render(<ModelEffortSelect providerId="p1" model="gpt-4o" providers={providers} value="off" onChange={vi.fn()} />)
    await act(async () => {})
    expect(screen.queryByRole('button', { name: '推理强度' })).toBeNull()
  })

  it('ignores late capabilities from a previously selected model', async () => {
    let finishOld: (value: ThinkingCapabilities) => void = () => {}
    vi.mocked(api.thinkingCapabilitiesForModel)
      .mockReturnValueOnce(new Promise<ThinkingCapabilities>((resolve) => { finishOld = resolve }))
      .mockResolvedValueOnce({ levels: ['low'], offMode: 'unsupported' })
    const { rerender } = render(<ModelEffortSelect providerId="p1" model="old" providers={providers} value="" onChange={vi.fn()} />)
    rerender(<ModelEffortSelect providerId="p1" model="new" providers={providers} value="" onChange={vi.fn()} />)
    const trigger = await screen.findByRole('button', { name: '推理强度' })
    await act(async () => finishOld({ levels: ['max'], offMode: 'supported' }))
    await userEvent.click(trigger)
    expect(screen.getByRole('option', { name: 'low' })).toBeTruthy()
    expect(screen.queryByRole('option', { name: 'max' })).toBeNull()
  })

  it('clears stale capabilities when lookup fails after provider edits', async () => {
    vi.mocked(api.thinkingCapabilitiesForModel)
      .mockResolvedValueOnce({ levels: ['high'], offMode: 'supported' })
      .mockRejectedValueOnce(new Error('offline'))
    const { rerender } = render(<ModelEffortSelect providerId="p1" model="model" providers={providers} value="" onChange={vi.fn()} />)
    await screen.findByRole('button', { name: '推理强度' })
    rerender(<ModelEffortSelect providerId="p1" model="model" providers={[makeProvider({ name: 'Updated' })]} value="" onChange={vi.fn()} />)
    await waitFor(() => expect(screen.queryByRole('button', { name: '推理强度' })).toBeNull())
  })
})
