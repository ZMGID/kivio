import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type OAuthDevicePrompt } from './tauri'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke,
  Channel: class { onmessage: (value: unknown) => void = () => {} },
}))

describe('connector OAuth cancellation boundary', () => {
  beforeEach(() => { invoke.mockReset() })

  it('delivers the device prompt and stops listening after completion', async () => {
    const prompt: OAuthDevicePrompt = { userCode: 'ABCD-EFGH', verificationUri: 'https://github.com/login/device', expiresIn: 900 }
    let complete!: (value: unknown) => void
    invoke.mockReturnValue(new Promise((resolve) => { complete = resolve }))
    const progress = vi.fn()
    const pending = api.connectorOauthConnect({ catalogId: 'github' }, progress)
    const channel = invoke.mock.calls[0][1].onDeviceCode
    channel.onmessage(prompt)
    expect(progress).toHaveBeenCalledWith(prompt)
    complete({ id: 'connector-github' })
    await expect(pending).resolves.toEqual({ id: 'connector-github' })
    channel.onmessage(prompt)
    expect(progress).toHaveBeenCalledTimes(1)
  })

  it('cancels the matching backend flow and discards late progress and credentials', async () => {
    let complete!: (value: unknown) => void
    invoke.mockImplementation((command: string) => command === 'connector_oauth_connect'
      ? new Promise((resolve) => { complete = resolve })
      : Promise.resolve())
    const controller = new AbortController()
    const progress = vi.fn()
    const pending = api.connectorOauthConnect({ catalogId: 'github' }, progress, controller.signal)
    const args = invoke.mock.calls[0][1]
    controller.abort()
    expect(invoke).toHaveBeenCalledWith('connector_oauth_cancel', { requestId: args.requestId })
    args.onDeviceCode.onmessage({ userCode: 'LATE' })
    expect(progress).not.toHaveBeenCalled()
    complete({ id: 'connector-github' })
    await expect(pending).rejects.toThrow('OAUTH_CANCELLED')
  })

  it('does not start an already cancelled request', async () => {
    const controller = new AbortController()
    controller.abort()
    await expect(api.connectorOauthConnect({ catalogId: 'github' }, undefined, controller.signal)).rejects.toThrow('OAUTH_CANCELLED')
    expect(invoke).not.toHaveBeenCalled()
  })
})
