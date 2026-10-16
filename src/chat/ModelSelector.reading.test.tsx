import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ModelSelector } from './ModelSelector'

vi.mock('../api/settingsCache', () => ({
  getSettingsCached: async () => ({
    providers: [{ id: 'reading-provider', name: 'Configured provider', enabled: true,
      enabledModels: ['custom-text', 'custom-vision-with-a-long-complete-model-id'], availableModels: [],
      modelOverrides: {
        'custom-text': { capabilities: { vision: false } },
        'custom-vision-with-a-long-complete-model-id': { capabilities: { vision: true } },
      },
    }], favoriteModels: ['reading-provider:custom-text'],
  }),
  subscribeSettings: () => () => {},
  setFavoriteModelsCached: vi.fn().mockResolvedValue(undefined),
}))

describe('shared ModelSelector reading options', () => {
  it('preserves the default Chat picker including non-vision models', async () => {
    render(<ModelSelector currentProviderId="reading-provider" currentModel="custom-text" onModelChange={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'custom-text' }))
    await waitFor(() => expect(screen.getAllByRole('button', { name: 'custom-text' })).toHaveLength(3))
    expect(await screen.findByRole('button', { name: 'custom-vision-with-a-long-complete-model-id' })).toBeVisible()
  })

  it('filters configured vision capability and favorites without shortening the selected model ID', async () => {
    const select = vi.fn()
    const model = 'custom-vision-with-a-long-complete-model-id'
    const { container } = render(<ModelSelector visionOnly placement="up" preserveLabel
      currentProviderId="reading-provider" currentModel={model} onModelChange={select} />)
    const trigger = screen.getByRole('button', { name: model })
    await waitFor(() => expect(trigger).toHaveAttribute('title', `Configured provider：${model}`))
    fireEvent.click(trigger)
    expect(screen.queryByText('custom-text')).toBeNull()
    expect(container.querySelector('.chat-model-selector-menu')).toHaveClass('bottom-full')
    expect(container.querySelector('.chat-model-selector--labeled')).not.toBeNull()
    fireEvent.click(screen.getAllByRole('button', { name: model })[1])
    expect(select).toHaveBeenCalledWith('reading-provider', model)
    expect(container.querySelector('.chat-model-selector-menu')).toBeNull()
  })
})
