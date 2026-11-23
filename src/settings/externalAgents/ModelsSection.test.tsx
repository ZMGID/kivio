import { act, render, screen } from '@testing-library/react'
import { invoke } from '@tauri-apps/api/core'
import { expect, it, vi } from 'vitest'
import { ModelsSection } from './ModelsSection'

vi.mock('@tauri-apps/api/core', async (importOriginal) => ({
  ...await importOriginal<typeof import('@tauri-apps/api/core')>(),
  invoke: vi.fn(),
}))
vi.mock('../../api/tauri', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../api/tauri')>(),
  isTauriRuntime: () => true,
}))

it('shows the current CLI snapshot while a probe runs and ignores the previous CLI result', async () => {
  let finish: (value: unknown) => void = () => {}
  vi.mocked(invoke).mockImplementation((_command, args) => {
    if (args && 'agentId' in args && args.agentId === 'codex') {
      return Promise.resolve({ models: [{ id: 'codex-cached', label: 'Codex cached' }], refreshing: false })
    }
    if (args && 'snapshotOnly' in args && args.snapshotOnly) {
      return Promise.resolve({ models: [{ id: 'claude-cached', label: 'Claude cached' }], refreshing: true })
    }
    return new Promise(resolve => { finish = resolve })
  })

  const props = { lang: 'en', customModels: [], onChange: vi.fn() } satisfies Omit<Parameters<typeof ModelsSection>[0], 'agentId'>
  const view = render(<ModelsSection {...props} agentId="claude" />)
  expect(await screen.findByText('claude-cached')).toBeInTheDocument()

  view.rerender(<ModelsSection {...props} agentId="codex" />)
  expect(screen.queryByText('claude-cached')).not.toBeInTheDocument()
  expect(await screen.findByText('codex-cached')).toBeInTheDocument()
  await act(async () => finish({ models: [{ id: 'claude-new', label: 'Claude new' }] }))
  expect(screen.queryByText('claude-new')).not.toBeInTheDocument()
  expect(screen.getByText('codex-cached')).toBeInTheDocument()
})
