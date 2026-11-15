import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, it, vi } from 'vitest'
import { ModelDetailDrawer } from './ModelDetailDrawer'
import { applyModelDatabaseSnapshot } from '../data/modelMatching'

it('saves a billing reference without changing the request model and can clear it', async () => {
  const save = vi.fn()
  const { rerender } = render(<ModelDetailDrawer modelName="gateway-model" lang="zh" onSave={save} onReset={vi.fn()} onClose={vi.fn()} />)
  const input = screen.getByRole('textbox', { name: '计费参考型号' })
  await userEvent.type(input, ' deepseek-flash ')
  await userEvent.click(screen.getByRole('button', { name: /保存/ }))
  expect(save).toHaveBeenLastCalledWith('gateway-model', expect.objectContaining({ pricingModel: 'deepseek-flash' }))
  rerender(<ModelDetailDrawer modelName="gateway-model" overrides={{ 'gateway-model': { pricingModel: 'deepseek-flash' } }} lang="zh" onSave={save} onReset={vi.fn()} onClose={vi.fn()} />)
  await userEvent.clear(input)
  await userEvent.click(screen.getByRole('button', { name: /保存/ }))
  expect(save).toHaveBeenLastCalledWith('gateway-model', expect.objectContaining({ pricingModel: undefined }))
})

it('refreshes untouched model defaults while preserving an open edited draft', async () => {
  const publish = (revision: number, contextWindow: number) => applyModelDatabaseSnapshot({
    revision, version: `2026-10-13.${revision}`, database: {
      'snapshot-model': { displayName: 'Snapshot model', contextWindow, capabilities: { vision: true } },
    },
  })
  publish(1, 100)
  const save = vi.fn()
  render(<ModelDetailDrawer modelName="snapshot-model" lang="zh" onSave={save} onReset={vi.fn()} onClose={vi.fn()} />)
  expect(screen.getByDisplayValue('100')).toBeTruthy()
  act(() => publish(2, 200))
  await waitFor(() => expect(screen.getByDisplayValue('200')).toBeTruthy())
  const name = screen.getByDisplayValue('Snapshot model')
  await userEvent.clear(name)
  await userEvent.type(name, 'My draft')
  act(() => publish(3, 300))
  expect(screen.getByDisplayValue('My draft')).toBeTruthy()
  expect(screen.getByDisplayValue('200')).toBeTruthy()
  await userEvent.click(screen.getByRole('button', { name: /保存/ }))
  expect(save).toHaveBeenCalledWith('snapshot-model', expect.objectContaining({ displayName: 'My draft', contextWindow: 200 }))
})

it('preserves billing references when catalog defaults exist without turning default prices into overrides', async () => {
  applyModelDatabaseSnapshot({ revision: 20, version: '2026-11-15.20', database: {
    'priced-model': { displayName: 'Priced model', contextWindow: 100, pricing: { input: 2, output: 8 } },
  } })
  const save = vi.fn()
  render(<ModelDetailDrawer modelName="priced-model" overrides={{ 'priced-model': { pricingModel: 'reference-model' } }} lang="zh" onSave={save} onReset={vi.fn()} onClose={vi.fn()} />)
  const reference = screen.getByRole('textbox', { name: '计费参考型号' })
  expect(reference).toHaveValue('reference-model')
  await userEvent.clear(reference)
  await userEvent.type(reference, 'new-reference')
  await userEvent.click(screen.getByRole('button', { name: /保存/ }))
  expect(save).toHaveBeenCalledWith('priced-model', expect.objectContaining({ pricingModel: 'new-reference', pricing: undefined }))
})

it('requires complete non-negative custom pricing and preserves explicit zero', async () => {
  const save = vi.fn()
  render(<ModelDetailDrawer modelName="unlisted-model" lang="zh" onSave={save} onReset={vi.fn()} onClose={vi.fn()} />)
  const [input, output] = screen.getAllByPlaceholderText('0.00')
  await userEvent.type(input, '0')
  expect(screen.getByRole('alert')).toHaveTextContent('同时填写输入和输出单价')
  expect(screen.getByRole('button', { name: /保存/ })).toBeDisabled()
  await userEvent.type(output, '0')
  await userEvent.click(screen.getByRole('button', { name: /保存/ }))
  expect(save).toHaveBeenCalledWith('unlisted-model', expect.objectContaining({ pricing: { input: 0, output: 0 } }))
})
