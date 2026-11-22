import { screen } from '@testing-library/react'
import { render } from '../test/renderWithNotices'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { SidebarAccountMenu } from './SidebarAccountMenu'

vi.mock('../api/tauri', () => ({
  api: {
    usageGetStats: vi.fn(),
    onDesktopPetEnabledChanged: vi.fn(async () => () => {}),
    desktopPetGetEnabled: vi.fn(async () => false),
    desktopPetSetEnabled: vi.fn(),
  },
}))

describe('SidebarAccountMenu usage', () => {
  it('shows today token total compactly and opens the usage page', async () => {
    vi.mocked(api.usageGetStats).mockResolvedValue({
      summary: { totalTokens: 1500 },
    } as Awaited<ReturnType<typeof api.usageGetStats>>)
    const onOpenUsage = vi.fn()
    const user = userEvent.setup()

    render(
      <SidebarAccountMenu
        triggerRect={{ left: 0, top: 200, width: 220 }}
        lang="zh"
        onSelectLang={vi.fn()}
        onOpenUsage={onOpenUsage}
        onClose={vi.fn()}
      />,
    )

    expect(await screen.findByText('1.5k')).toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /检查更新/ })).not.toBeInTheDocument()
    expect(api.usageGetStats).toHaveBeenCalledWith({ range: 'today', limit: 0 })

    await user.click(screen.getByRole('menuitem', { name: /模型用量/ }))
    expect(onOpenUsage).toHaveBeenCalledOnce()
  })

  it('clears a pet save failure once a retry succeeds', async () => {
    vi.mocked(api.usageGetStats).mockResolvedValue({ summary: { totalTokens: 0 } } as Awaited<ReturnType<typeof api.usageGetStats>>)
    vi.mocked(api.desktopPetSetEnabled)
      .mockRejectedValueOnce(new Error('ipc down'))
      .mockResolvedValueOnce(true)
    const user = userEvent.setup()
    render(
      <SidebarAccountMenu
        triggerRect={{ left: 0, top: 200, width: 220 }}
        lang="zh"
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        onClose={vi.fn()}
      />,
    )
    const toggle = await screen.findByRole('switch', { name: '桌面宠物' })
    await vi.waitFor(() => expect(toggle).toBeEnabled())
    await user.click(toggle)
    expect(await screen.findByRole('alert')).toHaveTextContent('未能切换桌宠')
    await user.click(toggle)
    await vi.waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument())
    expect(api.desktopPetSetEnabled).toHaveBeenCalledTimes(2)
  })
})
