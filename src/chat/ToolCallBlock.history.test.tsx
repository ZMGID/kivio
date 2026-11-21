import { act, render, screen } from '@testing-library/react'
import { StrictMode } from 'react'
import userEvent from '@testing-library/user-event'
import { invoke } from '@tauri-apps/api/core'
import { beforeEach, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { ToolCallBlock } from './ToolCallBlock'
import { i18n } from '../components/i18n'

vi.mock('@tauri-apps/api/core', async importActual => ({
  ...await importActual<typeof import('@tauri-apps/api/core')>(), invoke: vi.fn(),
}))
vi.mock('./utils', async importActual => ({
  ...await importActual<typeof import('./utils')>(), isTauriRuntime: () => true,
}))
beforeEach(() => vi.mocked(invoke).mockReset())

const historyMessage = { id: 'message', role: 'assistant', timestamp: 1, content: '', tool_calls: [{
  id: 'call', name: 'bash', status: 'success', arguments: { command: 'npm test' }, result_preview: null,
}] }
const windowResponse = { success: true, conversation: { messages: [historyMessage] },
  history_start: 1, history_total: 2, history_directory: [], read_ms: 1, prepare_ms: 1 }

it('reads only an open result, ignores late reads after navigation, and retries the current page reference', async () => {
  const user = userEvent.setup()
  const ipc = vi.mocked(invoke)
  ipc.mockResolvedValueOnce({ ...windowResponse, deferred_tool_results: { message: { call: 'old-hash' } } })
  const window = await chatApi.getConversationWindow('first')
  expect(ipc).toHaveBeenLastCalledWith('chat_get_conversation_window', { conversationId: 'first', deferToolResults: true })
  const { rerender } = render(<StrictMode><ToolCallBlock toolCall={window.messages[0].tool_calls![0]} /></StrictMode>)
  expect(ipc).toHaveBeenCalledTimes(1)
  let resolveOld!: (text: string) => void
  ipc.mockReturnValueOnce(new Promise<string>(resolve => { resolveOld = resolve }))
  await user.click(screen.getByRole('button', { name: /npm test/ }))
  expect(ipc).toHaveBeenLastCalledWith('chat_get_tool_result', {
    conversationId: 'first', messageId: 'message', toolCallId: 'call', expectedHash: 'old-hash',
  })
  expect(screen.getByRole('status')).toBeVisible()

  ipc.mockResolvedValueOnce({ success: true, messages: [historyMessage], deferred_tool_results: { message: { call: 'new-hash' } } })
  const page = await chatApi.getConversationPage('second', 1)
  expect(ipc).toHaveBeenLastCalledWith('chat_get_conversation_page', { conversationId: 'second', before: 1, deferToolResults: true })
  ipc.mockRejectedValueOnce(new Error('disk temporarily unavailable'))
  rerender(<StrictMode><ToolCallBlock toolCall={page.messages[0].tool_calls![0]} /></StrictMode>)
  await screen.findByText('disk temporarily unavailable')
  await act(async () => resolveOld('old output'))
  expect(screen.queryByText('old output')).toBeNull()

  ipc.mockResolvedValueOnce('stdout: {"answer":"latest output"}')
  await user.click(screen.getByRole('button', { name: i18n.zh.chatRetry }))
  expect(await screen.findByText('答: latest output')).toBeVisible()
  expect(ipc).toHaveBeenLastCalledWith('chat_get_tool_result', {
    conversationId: 'second', messageId: 'message', toolCallId: 'call', expectedHash: 'new-hash',
  })
  expect(screen.queryByRole('alert')).toBeNull()
  const tool = screen.getByRole('button', { name: /npm test/ })
  await user.click(tool)
  await user.click(tool)
  expect(await screen.findByText('答: latest output')).toBeVisible()
  expect(ipc).toHaveBeenCalledTimes(5)
})

it('renders an inline preview from an older backend without issuing a detail request', async () => {
  const ipc = vi.mocked(invoke)
  ipc.mockResolvedValueOnce({ ...windowResponse, conversation: { messages: [{ ...historyMessage,
    tool_calls: [{ ...historyMessage.tool_calls[0], result_preview: 'inline output' }],
  }] } })
  const window = await chatApi.getConversationWindow('legacy')
  render(<ToolCallBlock toolCall={window.messages[0].tool_calls![0]} defaultOpen />)
  expect(screen.getByText('inline output')).toBeVisible()
  expect(ipc).toHaveBeenCalledTimes(1)
})
