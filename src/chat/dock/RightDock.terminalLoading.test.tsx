import { act, render, screen, waitFor } from '@testing-library/react'
import type { ComponentProps } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { RightDock } from './RightDock'

// 独立文件：终端代码的加载状态属于模块级缓存，需要从“未加载”开始观察。
const { terminalLoads } = vi.hoisted(() => ({ terminalLoads: { count: 0 } }))

vi.mock('./FileTreePanel', () => ({ FileTreePanel: () => <div data-testid="files-panel" /> }))
vi.mock('./GitPanel', () => ({ GitPanel: () => <div data-testid="git-panel" /> }))
vi.mock('./PullRequestsPanel', () => ({ PullRequestsPanel: () => <div data-testid="pulls-panel" /> }))
vi.mock('./TerminalPanel', () => {
  terminalLoads.count += 1
  return {
    TerminalPanel: ({ active }: { active: boolean }) => <div data-testid="terminal-panel" data-active={active} />,
  }
})
vi.mock('./api', () => ({ dockApi: { resolveCwd: vi.fn(async () => '/tmp/project') } }))
vi.mock('../../api/settingsCache', () => ({ peekSettings: () => null, subscribeSettings: () => () => {} }))
vi.mock('../../api/tauri', () => ({ api: {
  chatListBackgroundTasks: vi.fn(async () => []),
  chatKillBackgroundCommand: vi.fn(),
  chatStopExternalBackgroundTask: vi.fn(),
  chatClearFinishedBackgroundTasks: vi.fn(),
} }))

function dockProps(overrides: Partial<ComponentProps<typeof RightDock>> = {}): ComponentProps<typeof RightDock> {
  return {
    open: true, width: 360, activeTab: 'files', openTabRequest: null, workdir: '/tmp/project', lang: 'zh',
    conversationId: 'conv-1', treeExpanded: [], revealRequest: null, previewRequest: null,
    onToggleTab: vi.fn(), onOpenTabRequestHandled: vi.fn(), onWidthChange: vi.fn(), onClose: vi.fn(),
    onTreeExpandedChange: vi.fn(), onRevealInTree: vi.fn(),
    ...overrides,
  }
}

describe('RightDock 终端按需加载', () => {
  it('开窗时不加载终端代码，空闲预取后点开终端同步显示', async () => {
    const props = dockProps()
    const { rerender } = render(<RightDock {...props} />)
    expect(terminalLoads.count).toBe(0)

    await waitFor(() => expect(terminalLoads.count).toBe(1), { timeout: 3000 })
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)))

    rerender(<RightDock {...props} activeTab="terminal" openTabRequest={{ tab: 'terminal' }} />)
    expect(screen.getByTestId('terminal-panel')).toHaveAttribute('data-active', 'true')
    expect(terminalLoads.count).toBe(1)
  })
})
