import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { i18n } from '../components/i18n'
import { WebSearchPanel } from './WebSearchPanel'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke,
  Channel: class {
    onmessage: (value: unknown) => void = () => {}
  },
}))

const pending: Array<(value: unknown) => void> = []

function oauthServer(token: string) {
  return {
    id: 'custom-tinyfish-mcp',
    name: 'TinyFish MCP',
    enabled: true,
    transport: 'streamable_http',
    url: 'https://agent.tinyfish.ai/mcp',
    command: '',
    args: [],
    env: {},
    headers: {},
    enabledTools: [],
    auth: { kind: 'oauth', accessToken: token, account: 'ada@tinyfish.ai' },
  }
}

function connectCalls() {
  return invoke.mock.calls.filter((call) => call[0] === 'connector_oauth_connect')
}

function openPanel(onChange = vi.fn()) {
  const view = render(
    <WebSearchPanel t={i18n.zh} lang="zh" webSearch={undefined} onChange={onChange} />,
  )
  fireEvent.click(screen.getByRole('button', { name: 'TinyFish MCP' }))
  return { ...view, onChange }
}

async function startAuth() {
  const button = screen.getByRole('button', { name: '授权 TinyFish' })
  await act(async () => {
    button.click()
  })
}

describe('TinyFish MCP authorization lifetime', () => {
  beforeEach(() => {
    pending.length = 0
    invoke.mockReset()
    invoke.mockImplementation((command: string) => {
      if (command === 'connector_oauth_connect') {
        return new Promise((resolve) => { pending.push(resolve) })
      }
      return Promise.resolve()
    })
  })

  it('cancels the backend flow on leave and does not save a late token', async () => {
    const { unmount, onChange } = openPanel()
    await startAuth()
    const requestId = connectCalls()[0][1].requestId as string
    unmount()

    expect(invoke).toHaveBeenCalledWith('connector_oauth_cancel', { requestId })
    await act(async () => {
      pending[0](oauthServer('late-token'))
    })
    expect(onChange).not.toHaveBeenCalled()
  })

  it('lets a later visit authorize without the cancelled result overwriting it', async () => {
    const onChange = vi.fn()
    const first = openPanel(onChange)
    await startAuth()
    const firstRequest = connectCalls()[0][1].requestId as string
    first.unmount()

    openPanel(onChange)
    await startAuth()
    expect(screen.getByRole('button', { name: '授权中…（请在浏览器完成）' })).toBeDisabled()
    expect(connectCalls()).toHaveLength(2)
    expect(connectCalls()[1][1].requestId).not.toBe(firstRequest)

    await act(async () => {
      pending[0](oauthServer('old-token'))
    })
    expect(onChange).not.toHaveBeenCalled()

    await act(async () => {
      pending[1](oauthServer('new-token'))
    })
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith({
      tinyfishMcpAuth: expect.objectContaining({ accessToken: 'new-token', account: 'ada@tinyfish.ai' }),
    })
    const cancellations = invoke.mock.calls.filter((call) => call[0] === 'connector_oauth_cancel')
    expect(cancellations.map((call) => call[1].requestId)).toEqual([firstRequest])
  })

  it('ignores a second authorize click before the button rerenders', async () => {
    openPanel()
    const button = screen.getByRole('button', { name: '授权 TinyFish' })
    await act(async () => {
      button.click()
      button.click()
    })
    expect(connectCalls()).toHaveLength(1)
  })
})
