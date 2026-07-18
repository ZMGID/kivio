import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ConversationContextMenu } from './ConversationContextMenu'

function renderMenu(lang: 'zh' | 'en', onExport = vi.fn(), onClose = vi.fn()) {
  render(
    <ConversationContextMenu
      anchor={{ left: 0, top: 0 }}
      projects={[]}
      sets={[]}
      lang={lang}
      onRename={vi.fn()}
      onExport={onExport}
      onMoveToProject={vi.fn()}
      onMoveToSet={vi.fn()}
      onDelete={vi.fn()}
      onClose={onClose}
    />,
  )
  return { onExport, onClose }
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

  it('renders the English action', () => {
    renderMenu('en')
    expect(screen.getByRole('menuitem', { name: 'Export' })).toBeInTheDocument()
  })
})
