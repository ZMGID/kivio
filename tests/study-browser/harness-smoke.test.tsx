import { fireEvent, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
vi.mock('@xterm/xterm', () => ({ Terminal: class {} }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class {} }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }) }))
import 'fake-indexeddb/auto'

it('boots the browser fixture through actual Chat and sends through the common owner', async () => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  document.body.innerHTML = '<div id="root"></div>'
  location.hash = '#chat/study'
  await import('./main')
  expect(await screen.findByRole('heading', { name: 'Kivio Study' })).toBeVisible()
  expect(document.querySelector('.chat-window-shell')).not.toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Normal Chat', exact: true }))
  const editor = await screen.findByRole('textbox')
  fireEvent.paste(editor, { clipboardData: { files: [], getData: (kind: string) => kind === 'text/plain' ? 'Smoke test question' : '' } })
  fireEvent.keyDown(editor, { key: 'Enter', code: 'Enter' })
  await waitFor(() => expect(window.__studyTest.requests).toHaveLength(1))
  await waitFor(() => expect(window.__studyTest.packets).toContain('run_completed'))
  expect(window.__studyTest.requests[0].studySource).toBeUndefined()
  expect(await screen.findByText('Simulated shared Chat reply: explain the supplied source and distinguish it from your interpretation.', {}, { timeout: 10000 })).toBeVisible()
})
