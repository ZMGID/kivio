import { useState } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ModelSelector } from './ModelSelector'

const settings = vi.hoisted(() => ({
  providers: [{ id: 'provider', name: 'Provider', enabled: true, enabledModels: ['model-a', 'model-b'], availableModels: [] as string[] }],
  favoriteModels: [] as string[],
}))
vi.mock('../api/settingsCache', () => ({
  getSettingsCached: async () => settings,
  subscribeSettings: () => () => {}, setFavoriteModelsCached: vi.fn(),
}))
vi.mock('../components/ModelIcon', () => ({ ModelIcon: () => null }))
vi.mock('../components/i18n', () => ({ useT: () => ({ chatFavorite: '收藏', chatUnfavorite: '取消收藏', chatSearchModels: '搜索模型', chatNoMatchingModels: '没有匹配的模型' }) }))
afterEach(() => {
  cleanup()
  settings.providers[0].enabledModels = ['model-a', 'model-b']
})

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

it('filters a long model list from the search box and picks the first match on Enter', async () => {
  settings.providers[0].enabledModels = Array.from({ length: 9 }, (_, i) => `model-${i}`).concat('special-model')
  const onModelChange = vi.fn()
  render(<ModelSelector currentProviderId="provider" currentModel="model-0" onModelChange={onModelChange} />)
  fireEvent.click(screen.getByRole('button', { name: 'model-0' }))
  const search = await screen.findByRole('textbox', { name: '搜索模型' })
  expect(screen.getByRole('button', { name: 'model-0', current: true })).toBeInTheDocument()
  fireEvent.change(search, { target: { value: 'SPECIAL' } })
  expect(screen.queryByRole('button', { name: 'model-3' })).not.toBeInTheDocument()
  fireEvent.keyDown(search, { key: 'Enter' })
  expect(onModelChange).toHaveBeenCalledWith('provider', 'special-model')
})

it('hides the search box for a short model list', async () => {
  render(<ModelSelector currentProviderId="provider" currentModel="model-a" onModelChange={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: 'model-a' }))
  await screen.findByRole('button', { name: 'model-b' })
  expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
})
