import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ConversationContextMenu } from './ConversationContextMenu'
import { i18n } from '../components/i18n'

it.each(['project', 'set'] as const)('keeps a large %s submenu inside the viewport and selects its last item', async (kind) => {
  const onMove = vi.fn()
  const onClose = vi.fn()
  const items = Array.from({ length: 30 }, (_, index) => ({ id: String(index), name: `Item ${index}`, created_at: 0, updated_at: 0 }))
  const bounds = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    return new DOMRect(900, 700, 176, this.getAttribute('aria-label') ? 420 : 200)
  })
  try {
    render(<ConversationContextMenu anchor={{ left: 900, top: 700 }} projects={items} sets={items} lang="zh"
      onRegenerateTitle={vi.fn()} onExport={vi.fn()} onMoveToProject={onMove} onMoveToSet={onMove}
      onDelete={vi.fn()} onClose={onClose} />)
    const label = kind === 'project' ? i18n.zh.chatAddToProject : i18n.zh.chatMoveToSet
    fireEvent.click(screen.getByRole('menuitem', { name: label }))
    const submenu = screen.getByRole('menu', { name: label })
    expect(Number.parseFloat(submenu.style.top) + 420).toBeLessThanOrEqual(window.innerHeight - 8)
    expect(Number.parseFloat(submenu.style.left) + 176).toBeLessThanOrEqual(window.innerWidth - 8)
    expect(submenu).toHaveClass('overflow-y-auto', 'custom-scrollbar')
    fireEvent.mouseLeave(screen.getByRole('menuitem', { name: label }))
    fireEvent.mouseDown(screen.getByText('Item 29'))
    expect(onClose).not.toHaveBeenCalled()
    fireEvent.click(screen.getByText('Item 29'))
    expect(onMove).toHaveBeenCalledWith('29')
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
  } finally {
    bounds.mockRestore()
  }
})

function renderMenu(
  lang: 'zh' | 'en',
  onExport = vi.fn(),
  onClose = vi.fn(),
  extra: {
    onRegenerateTitle?: () => void
    canRegenerateTitle?: boolean
    showNativeSession?: boolean
    nativeSessionId?: string | null
    nativeSessionLoading?: boolean
  } = {},
) {
  render(
    <ConversationContextMenu
      anchor={{ left: 0, top: 0 }}
      projects={[]}
      sets={[]}
      lang={lang}
      canRegenerateTitle={extra.canRegenerateTitle}
      onRegenerateTitle={extra.onRegenerateTitle ?? vi.fn()}
      onExport={onExport}
      onMoveToProject={vi.fn()}
      onMoveToSet={vi.fn()}
      onDelete={vi.fn()}
      onClose={onClose}
      showNativeSession={extra.showNativeSession}
      nativeSessionId={extra.nativeSessionId}
      nativeSessionLoading={extra.nativeSessionLoading}
    />,
  )
  return { onExport, onClose, onRegenerateTitle: extra.onRegenerateTitle }
}

describe('ConversationContextMenu export', () => {
  it('renders the localized Chinese action and closes after export', async () => {
    const user = userEvent.setup()
    const { onExport, onClose } = renderMenu('zh')
    await user.click(screen.getByRole('menuitem', { name: '导出' }))
    expect(onExport).toHaveBeenCalledOnce()
    // 关闭在退场动画结束后触发（useCloseAnimation：animationend / 超时兜底）
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
  })

})

describe('ConversationContextMenu popout', () => {
  it('renders the localized Chinese action and closes after opening', async () => {
    const user = userEvent.setup()
    const onOpenInPopout = vi.fn()
    const onClose = vi.fn()
    render(
      <ConversationContextMenu
        anchor={{ left: 0, top: 0 }}
        projects={[]}
        sets={[]}
        lang="zh"
        onRegenerateTitle={vi.fn()}
        onExport={vi.fn()}
        onMoveToProject={vi.fn()}
        onMoveToSet={vi.fn()}
        onOpenInPopout={onOpenInPopout}
        onDelete={vi.fn()}
        onClose={onClose}
      />,
    )
    await user.click(screen.getByRole('menuitem', { name: '在新窗口打开' }))
    expect(onOpenInPopout).toHaveBeenCalledOnce()
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
  })

  it('hides the action when no handler is provided', () => {
    renderMenu('zh')
    expect(screen.queryByRole('menuitem', { name: '在新窗口打开' })).not.toBeInTheDocument()
  })
})

describe('ConversationContextMenu regenerate title', () => {
  it('renders the localized Chinese action and closes after regenerate', async () => {
    const user = userEvent.setup()
    const onRegenerateTitle = vi.fn()
    const onClose = vi.fn()
    renderMenu('zh', vi.fn(), onClose, { onRegenerateTitle })
    await user.click(screen.getByRole('menuitem', { name: '重新生成标题' }))
    expect(onRegenerateTitle).toHaveBeenCalledOnce()
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
  })

  it('disables regenerate when the conversation has no messages', () => {
    renderMenu('zh', vi.fn(), vi.fn(), { canRegenerateTitle: false })
    expect(screen.getByRole('menuitem', { name: '重新生成标题' })).toBeDisabled()
  })
})

describe('ConversationContextMenu removed actions', () => {
  it('does not offer rename or pin', () => {
    renderMenu('zh')
    expect(screen.queryByRole('menuitem', { name: '重命名' })).not.toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: '置顶聊天' })).not.toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /原生会话/ })).not.toBeInTheDocument()
  })
})

describe('ConversationContextMenu native session', () => {
  it('shows the unbound native session id for local CLI conversations', () => {
    renderMenu('zh', vi.fn(), vi.fn(), { showNativeSession: true, nativeSessionId: null })
    const item = screen.getByRole('menuitem', { name: /原生会话/ })
    expect(item).toHaveTextContent('尚未绑定')
    expect(item).toBeDisabled()
  })

  it('copies the bound native session id', async () => {
    const user = userEvent.setup()
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    const onClose = vi.fn()
    const sessionId = '0194abcd-61e2-7113-b077-58d3d91fb3d7'
    renderMenu('zh', vi.fn(), onClose, { showNativeSession: true, nativeSessionId: sessionId })
    await user.click(screen.getByRole('menuitem', { name: /原生会话/ }))
    expect(writeText).toHaveBeenCalledWith(sessionId)
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
  })

})
