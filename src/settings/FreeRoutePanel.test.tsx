import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { FreeRoutePanel } from './FreeRoutePanel'
import { singleVendorApiKey, type FreeRouteConfig } from '../api/tauri'

vi.mock('../api/tauri', async () => {
  const actual = await vi.importActual<typeof import('../api/tauri')>('../api/tauri')
  return { ...actual, api: { openExternal: vi.fn() } }
})

function config(overrides: Partial<FreeRouteConfig> = {}): FreeRouteConfig {
  return {
    vendors: [
      { id: 'opencode', enabled: true, apiKey: '' },
      { id: 'openrouter', enabled: true, apiKey: '' },
      { id: 'sensenova', enabled: false, apiKey: '' },
      { id: 'groq', enabled: true, apiKey: 'sk-groq' },
    ],
    ...overrides,
  }
}

describe('FreeRoutePanel', () => {
  it('does not show a key field for anonymous OpenCode Zen', () => {
    render(
      <FreeRoutePanel
        config={config()}
        lang="zh"
        revealedKeys={new Set()}
        onChange={vi.fn()}
        onToggleKeyReveal={vi.fn()}
      />,
    )
    expect(screen.getByText('OpenCode Zen')).toBeInTheDocument()
    expect(screen.getByText(/无需 API Key/)).toBeInTheDocument()
  })

  it('keeps only the first pasted key for a vendor', async () => {
    const onChange = vi.fn()
    render(
      <FreeRoutePanel
        config={config()}
        lang="en"
        revealedKeys={new Set(['freeroute-openrouter'])}
        onChange={onChange}
        onToggleKeyReveal={vi.fn()}
      />,
    )
    const input = screen.getAllByPlaceholderText('Exactly one key')[0]
    fireEvent.change(input, { target: { value: 'sk-one,sk-two' } })
    const last = onChange.mock.calls.at(-1)?.[0] as FreeRouteConfig
    const openrouter = last.vendors.find((vendor) => vendor.id === 'openrouter')
    expect(openrouter?.apiKey).toBe('sk-one')
    expect(singleVendorApiKey('sk-one,sk-two')).toBe('sk-one')
  })
})
