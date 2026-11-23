import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'

import { i18n } from '../components/i18n'
import { WebSearchPanel } from './WebSearchPanel'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  Channel: class { onmessage = () => {} },
}))

describe('DeepSeek search settings', () => {
  it('offers Search and Fetch with the existing credential and base URL, without model settings', () => {
    const onChange = vi.fn()
    render(<WebSearchPanel t={i18n.zh} onChange={onChange} webSearch={{
      enabled: true, provider: 'exa', tavilyApiKey: '', exaApiKey: 'exa-key',
      deepseekApiKey: 'existing-key', deepseekBaseUrl: 'https://relay.example/v1',
      maxResults: 5, searchDepth: 'basic',
    }} />)
    fireEvent.click(screen.getByRole('button', { name: 'DeepSeek' }))
    expect(screen.getByDisplayValue('existing-key')).toBeTruthy()
    expect(screen.getByDisplayValue('https://relay.example/v1')).toBeTruthy()
    expect(screen.queryByText(i18n.zh.webSearchModel)).toBeNull()
    expect(screen.queryByText(i18n.zh.webSearchSystemPrompt)).toBeNull()
    fireEvent.change(screen.getByDisplayValue('https://relay.example/v1'), { target: { value: 'https://api.deepseek.com' } })
    expect(onChange).toHaveBeenLastCalledWith({ deepseekBaseUrl: 'https://api.deepseek.com' })
    fireEvent.click(screen.getByRole('button', { name: i18n.zh.webSearchDefaultsNav }))
    fireEvent.click(screen.getByRole('button', { name: i18n.zh.webSearchFetchFollow.replace('{name}', 'Exa') }))
    fireEvent.click(screen.getByRole('option', { name: 'DeepSeek' }))
    expect(onChange).toHaveBeenLastCalledWith({ fetchProvider: 'deepseek' })
  })

  it('searches with the selected image alone and clears it when the default provider changes', async () => {
    const config = { enabled: true, provider: 'deepseek' as const, tavilyApiKey: '', exaApiKey: '', maxResults: 5, searchDepth: 'basic' as const }
    const { rerender } = render(<WebSearchPanel t={i18n.zh} onChange={vi.fn()} webSearch={config} />)
    vi.mocked(open).mockResolvedValueOnce('/tmp/logo.png')
    vi.mocked(invoke).mockResolvedValueOnce({ success: true, results: [
      { title: 'Logo source', url: 'https://example.com/logo', imageUrl: 'https://example.com/logo.png' },
    ] })
    fireEvent.click(screen.getByRole('button', { name: i18n.zh.webSearchChooseImage }))
    await screen.findByText('logo.png')
    fireEvent.click(screen.getByRole('button', { name: i18n.zh.webSearchTestRun }))
    expect((await screen.findByRole('link', { name: i18n.zh.webSearchOpenImage })).getAttribute('href')).toBe('https://example.com/logo.png')
    expect(invoke).toHaveBeenCalledWith('test_web_search', expect.objectContaining({ query: '', imagePath: '/tmp/logo.png' }))
    rerender(<WebSearchPanel t={i18n.zh} onChange={vi.fn()} webSearch={{ ...config, provider: 'exa' }} />)
    expect(screen.queryByText('logo.png')).toBeNull()
    expect(screen.getByRole('button', { name: i18n.zh.webSearchTestRun })).toHaveProperty('disabled', true)
  })

  it('assigns a provider to search or fetch from its detail page', () => {
    const onChange = vi.fn()
    render(<WebSearchPanel t={i18n.zh} onChange={onChange} webSearch={{
      enabled: true, provider: 'exa', fetchProvider: 'deepseek', tavilyApiKey: '', exaApiKey: 'exa-key',
      deepseekApiKey: 'existing-key', maxResults: 5, searchDepth: 'basic',
    }} />)
    const nav = within(screen.getByRole('navigation'))
    fireEvent.click(nav.getByRole('button', { name: 'DeepSeek' }))
    // DeepSeek already serves fetch, so only the search role offers an action.
    const [useForSearch, ...rest] = screen.getAllByRole('button', { name: i18n.zh.webSearchSetDefault })
    expect(rest).toHaveLength(0)
    expect(screen.getByText(i18n.zh.webSearchInUse)).toBeTruthy()
    fireEvent.click(useForSearch)
    expect(onChange).toHaveBeenLastCalledWith({ provider: 'deepseek', fetchProvider: null })

    fireEvent.click(nav.getByRole('button', { name: 'Brave' }))
    expect(screen.getByText(i18n.zh.webSearchFetchUnsupported)).toBeTruthy()
    expect(screen.getAllByRole('button', { name: i18n.zh.webSearchSetDefault })).toHaveLength(1)
  })
})
