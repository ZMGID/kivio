import { cleanup, fireEvent, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { render } from '../test/renderWithNotices'
import { OAuthDeviceDialog } from './ConnectorOAuth'
import { i18n } from '../components/i18n'
vi.mock('../api/tauri', () => ({ api: {
  chatWriteClipboardText: vi.fn().mockRejectedValue(new Error('clipboard unavailable')),
  openExternal: vi.fn().mockRejectedValue(new Error('browser unavailable')),
} }))
afterEach(cleanup)
it.each([
  [i18n.zh.connectorsDeviceCopy, i18n.zh.connectorsDeviceCopyFailed],
  [i18n.zh.connectorsDeviceOpen, i18n.zh.connectorsDeviceOpenFailed],
])('reports a repeated failure after dismissing %s', async (label, error) => {
  render(<OAuthDeviceDialog prompt={{ userCode: 'ABCD', verificationUri: 'https://example.com', expiresIn: 300 }} onCancel={vi.fn()} lang="zh" />)
  fireEvent.click(screen.getByRole('button', { name: label }))
  expect(await screen.findByRole('alert')).toHaveTextContent(error)
  fireEvent.click(screen.getByRole('button', { name: '关闭提示' }))
  fireEvent.click(screen.getByRole('button', { name: label }))
  expect(await screen.findByRole('alert')).toHaveTextContent(error)
})
