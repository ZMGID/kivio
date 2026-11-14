import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { SessionCenter } from './SessionCenter'
import type { ChatProject, ConversationSearchHit } from './types'

const project: ChatProject = {
  id: 'project-1',
  name: '项目一',
  created_at: 1,
  updated_at: 1,
}

const conversation: ConversationSearchHit = {
  id: 'conversation-1',
  title: '历史对话',
  preview: '预览',
  provider_id: 'provider',
  model: 'model',
  message_count: 2,
  created_at: 1,
  updated_at: 1,
  project_id: project.id,
  folder: project.name,
}

async function renderCenter(onSelectConversation = vi.fn()) {
  vi.spyOn(chatApi, 'queryConversations').mockResolvedValue({ items: [conversation], total: 1 })
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([])

  const view = render(
    <SessionCenter
      lang="zh"
      onSelectConversation={onSelectConversation}
    />,
  )
  await screen.findByText('历史对话')
  return { ...view, onSelectConversation }
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('SessionCenter conversation navigation', () => {
  it('passes the target project scope as part of the conversation selection', async () => {
    const user = userEvent.setup()
    const { onSelectConversation } = await renderCenter()

    await user.click(screen.getByRole('button', { name: /历史对话/ }))

    expect(onSelectConversation).toHaveBeenCalledWith(
      conversation.id,
      conversation,
      { project, set: null },
    )
  })
})

describe('SessionCenter row menu', () => {
  it('portals the menu outside the clipped settings container', async () => {
    const user = userEvent.setup()
    const { container } = await renderCenter()
    const row = screen.getByRole('button', { name: /历史对话/ })
    const trigger = row.querySelector('button[data-row-chrome]')
    expect(trigger).toBeTruthy()

    await user.click(trigger as HTMLButtonElement)

    const menuItem = await screen.findByRole('menuitem', { name: '收藏' })
    const menu = menuItem.closest('[role="menu"]')
    expect(menu).toBeTruthy()
    expect(menu?.parentElement).toBe(document.body)
    expect(container.contains(menu)).toBe(false)
    await waitFor(() => expect(menu).toBeVisible())
  })
})

describe('SessionCenter toolbar', () => {
  it('searches message bodies by default and offers only the three scopes', async () => {
    await renderCenter()
    expect(chatApi.queryConversations).toHaveBeenLastCalledWith(
      expect.objectContaining({ shelf: 'all', fullText: true, sort: 'updated', order: 'desc' }),
    )
    const scope = screen.getByRole('group', { name: '范围' })
    expect(Array.from(scope.querySelectorAll('button'), (button) => button.textContent)).toEqual(['全部', '收藏', '归档'])
    expect(screen.getByText('共 1 条')).toBeInTheDocument()
  })

  it('switches scope and filters by owner in the same query', async () => {
    const user = userEvent.setup()
    await renderCenter()

    await user.click(screen.getByRole('button', { name: '归档' }))
    await waitFor(() => expect(chatApi.queryConversations).toHaveBeenLastCalledWith(
      expect.objectContaining({ shelf: 'archived', projectId: null, setId: null }),
    ))

    await user.click(screen.getByRole('button', { name: '归属' }))
    await user.click(await screen.findByRole('option', { name: project.name }))
    await waitFor(() => expect(chatApi.queryConversations).toHaveBeenLastCalledWith(
      expect.objectContaining({ shelf: 'archived', projectId: project.id, setId: null }),
    ))
  })

  it('shows the owner on each row and groups time-sorted rows by day', async () => {
    await renderCenter()
    const row = screen.getByRole('button', { name: /历史对话/ })
    expect(row).toHaveTextContent(project.name)
    expect(screen.getByText('更早')).toBeInTheDocument()
  })
})
