import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { ConversationBranches } from './ConversationBranches'
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

it('loads on open, switches the chosen app branch, and closes only on confirmation', async () => {
  const onSwitch = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true)
  render(<ConversationBranches conversationId="c1" disabled={false} lang="zh" onSwitch={onSwitch} />)
  expect(chatApi.getBranches).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: '历史分支' }))
  const dialog = screen.getByRole('dialog', { name: '历史分支' })
  fireEvent.click(await within(dialog).findByRole('button', { name: '选择分支' }))
  fireEvent.click(screen.getByRole('option', { name: '编辑后' }))
  fireEvent.click(within(dialog).getByRole('button', { name: '切换分支' }))
  await waitFor(() => expect(onSwitch).toHaveBeenCalledWith('app-2'))
  expect(dialog).toBeInTheDocument()
  await waitFor(() => expect(within(dialog).getByRole('button', { name: '切换分支' })).toBeEnabled())
  fireEvent.click(within(dialog).getByRole('button', { name: '切换分支' }))
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
})

it('does not open while disabled and disables a dialog when the conversation becomes busy', async () => {
  const onSwitch = vi.fn()
  const view = render(<ConversationBranches conversationId="c1" disabled lang="zh" onSwitch={onSwitch} />)
  fireEvent.click(screen.getByRole('button', { name: '历史分支' }))
  expect(chatApi.getBranches).not.toHaveBeenCalled()
  view.rerender(<ConversationBranches conversationId="c1" disabled={false} lang="zh" onSwitch={onSwitch} />)
  fireEvent.click(screen.getByRole('button', { name: '历史分支' }))
  await screen.findByRole('button', { name: '选择分支' })
  view.rerender(<ConversationBranches conversationId="c1" disabled lang="zh" onSwitch={onSwitch} />)
  expect(screen.getByRole('button', { name: '选择分支' })).toBeDisabled()
  expect(screen.getByRole('button', { name: '切换分支' })).toBeDisabled()
})

it('ignores the old conversation branch list after navigation', async () => {
  let resolve: (value: typeof branches) => void = () => {}
  vi.mocked(chatApi.getBranches).mockReturnValueOnce(new Promise(done => { resolve = done }))
  const onSwitch = vi.fn()
  const view = render(<ConversationBranches key="c1" conversationId="c1" disabled={false} lang="zh" onSwitch={onSwitch} />)
  fireEvent.click(screen.getByRole('button', { name: '历史分支' }))
  view.rerender(<ConversationBranches key="c2" conversationId="c2" disabled={false} lang="zh" onSwitch={onSwitch} />)
  await act(async () => { resolve(branches) })
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '历史分支' }))
  await screen.findByRole('button', { name: '选择分支' })
  expect(chatApi.getBranches).toHaveBeenLastCalledWith('c2')
})
