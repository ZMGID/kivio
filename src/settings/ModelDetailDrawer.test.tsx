import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, it, vi } from 'vitest'
import { ModelDetailDrawer } from './ModelDetailDrawer'
import { applyModelDatabaseSnapshot } from '../data/modelMatching'

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
