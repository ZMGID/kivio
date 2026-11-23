import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { i18n } from '../components/i18n'
import { WebSearchPanel } from './WebSearchPanel'

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
