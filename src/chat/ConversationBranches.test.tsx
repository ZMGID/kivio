import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { ConversationBranchDialog } from './ConversationBranches'
import { chatApi } from './api'

vi.mock('./api', () => ({ chatApi: { getBranches: vi.fn() } }))
const branches = [
  { id: 'app-1', label: '原始分支', active: true },
  { id: 'app-2', label: '编辑后', active: false },
]

beforeEach(() => {
  vi.mocked(chatApi.getBranches).mockReset().mockResolvedValue(branches)
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
})

it('switches the chosen app branch and closes only on confirmation', async () => {
  const onSwitch = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true)
  const onClose = vi.fn()
  render(<ConversationBranchDialog conversationId="c1" disabled={false} lang="zh" onSwitch={onSwitch} onClose={onClose} />)
  const dialog = screen.getByRole('dialog', { name: '历史分支' })
  fireEvent.click(await within(dialog).findByRole('button', { name: '选择分支' }))
  fireEvent.click(screen.getByRole('option', { name: '编辑后' }))
  fireEvent.click(within(dialog).getByRole('button', { name: '切换分支' }))
  await waitFor(() => expect(onSwitch).toHaveBeenCalledWith('app-2'))
  await waitFor(() => expect(within(dialog).getByRole('button', { name: '切换分支' })).toBeEnabled())
  expect(onClose).not.toHaveBeenCalled()
  fireEvent.click(within(dialog).getByRole('button', { name: '切换分支' }))
  await waitFor(() => expect(onClose).toHaveBeenCalled())
})

it('disables switching while the conversation is busy', async () => {
  const props = { conversationId: 'c1', lang: 'zh' as const, onSwitch: vi.fn(), onClose: vi.fn() }
  const view = render(<ConversationBranchDialog {...props} disabled={false} />)
  await screen.findByRole('button', { name: '选择分支' })
  view.rerender(<ConversationBranchDialog {...props} disabled />)
  expect(screen.getByRole('button', { name: '选择分支' })).toBeDisabled()
  expect(screen.getByRole('button', { name: '切换分支' })).toBeDisabled()
})

it('ignores the old conversation branch list after navigation', async () => {
  let resolve: (value: typeof branches) => void = () => {}
  vi.mocked(chatApi.getBranches).mockReturnValueOnce(new Promise(done => { resolve = done }))
  const props = { disabled: false, lang: 'zh' as const, onSwitch: vi.fn(), onClose: vi.fn() }
  const view = render(<ConversationBranchDialog key="c1" conversationId="c1" {...props} />)
  view.rerender(<ConversationBranchDialog key="c2" conversationId="c2" {...props} />)
  await act(async () => { resolve([{ id: 'stale', label: '旧对话', active: false }]) })
  await screen.findByRole('button', { name: '选择分支' })
  expect(screen.queryByText('旧对话')).not.toBeInTheDocument()
  expect(chatApi.getBranches).toHaveBeenLastCalledWith('c2')
})
