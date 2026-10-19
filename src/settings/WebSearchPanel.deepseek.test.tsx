import { fireEvent, render, screen } from '@testing-library/react'
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
    render(<WebSearchPanel t={i18n.zh} lang="zh" onChange={onChange} webSearch={{
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
})
