import { describe, expect, it, vi } from 'vitest'
import { api } from './tauri'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async () => {}) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

describe('HTML render opening', () => {
  it('passes the owning conversation, attachment and current theme to the backend', async () => {
    await api.openHtmlRender('conv_1', 'htmlrender-12345678-1234-1234-1234-123456789abc', ':root{--foreground:#123456}')
    expect(invoke).toHaveBeenCalledWith('open_html_render', {
      conversationId: 'conv_1',
      attachmentId: 'htmlrender-12345678-1234-1234-1234-123456789abc',
      themeCss: ':root{--foreground:#123456}',
    })
  })
})
