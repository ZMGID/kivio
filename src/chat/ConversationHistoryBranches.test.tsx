import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { ConversationHistoryBranches } from './ConversationHistoryBranches'

vi.mock('./api', () => ({ chatApi: { getBranches: vi.fn() } }))

const branches = [
  { id: 'b1', label: 'Original', active: true },
  { id: 'b2', label: 'Earlier', active: false },
]

beforeEach(() => vi.mocked(chatApi.getBranches).mockReset())

it('discovers a newly created branch when the same conversation revision changes', async () => {
  vi.mocked(chatApi.getBranches).mockResolvedValueOnce(branches.slice(0, 1)).mockResolvedValueOnce(branches)
  const props = { conversationId: 'c1', lang: 'zh' as const, disabled: false, onSwitch: vi.fn() }
  const view = render(<ConversationHistoryBranches {...props} revision={1} />)
  await act(async () => {})
  expect(screen.queryByRole('button', { name: '历史分支' })).toBeNull()
  view.rerender(<ConversationHistoryBranches {...props} revision={2} />)
  expect(await screen.findByRole('button', { name: '历史分支' })).toBeEnabled()
})

it('blocks another selection while a branch switch is awaiting persistence', async () => {
  vi.mocked(chatApi.getBranches).mockResolvedValue(branches)
  let finish: (value: boolean) => void = () => {}
  const onSwitch = vi.fn(() => new Promise<boolean>(resolve => { finish = resolve }))
  render(<ConversationHistoryBranches conversationId="c1" lang="zh" disabled={false} onSwitch={onSwitch} />)
  fireEvent.click(await screen.findByRole('button', { name: '历史分支' }))
  fireEvent.click(screen.getByRole('menuitemradio', { name: 'Earlier' }))
  fireEvent.click(screen.getByRole('button', { name: '历史分支' }))
  expect(screen.getByRole('menuitemradio', { name: 'Earlier' })).toBeDisabled()
  expect(onSwitch).toHaveBeenCalledOnce()
  await act(async () => { finish(false) })
  expect(screen.getByRole('menuitemradio', { name: 'Earlier' })).toBeEnabled()
})
