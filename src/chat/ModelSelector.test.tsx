import { useState } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ModelSelector } from './ModelSelector'

vi.mock('../api/settingsCache', () => ({
  getSettingsCached: async () => ({ providers: [{ id: 'provider', name: 'Provider', enabled: true, enabledModels: ['model-a', 'model-b'], availableModels: [] }], favoriteModels: [] }),
  subscribeSettings: () => () => {}, setFavoriteModelsCached: vi.fn(),
}))
vi.mock('../components/ModelIcon', () => ({ ModelIcon: () => null }))
vi.mock('../components/i18n', () => ({ useT: () => ({ chatFavorite: '收藏', chatUnfavorite: '取消收藏' }) }))
afterEach(cleanup)

it('keeps a panel-hosted model menu outside clipping ancestors and commits a selected model', async () => {
  function Panel() {
    const [model, setModel] = useState('model-a')
    return <section aria-label="翻译设置" style={{ overflow: 'hidden', height: 32 }}>
      <ModelSelector portal currentProviderId="provider" currentModel={model} onModelChange={(_, value) => setModel(value)} />
    </section>
  }
  render(<Panel />)
  fireEvent.click(screen.getByRole('button', { name: 'model-a' }))
  const choice = await screen.findByRole('button', { name: 'model-b' })
  expect(screen.getByRole('region', { name: '翻译设置' })).not.toContainElement(choice)
  fireEvent.click(choice)
  expect(screen.getByRole('button', { name: 'model-b' })).toHaveAttribute('aria-expanded', 'false')
  expect(screen.queryByRole('button', { name: 'model-a' })).not.toBeInTheDocument()
})
