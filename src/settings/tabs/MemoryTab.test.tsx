import { render } from '../../test/renderWithNotices'
import { screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MemoryTab } from './MemoryTab'

/**
 * 回归重点：
 *   1. L1 / L2 两个编辑器的 value / savedValue / maxBytes 不串（同类型，接错不报错）
 *   2. onDraftChange 带上正确的 layer（shell 侧靠它写对 key，并清 success 提示）
 *   3. L1 超字节上限时禁用保存
 */
type Props = Parameters<typeof MemoryTab>[0]

function renderTab(overrides: Partial<Props> = {}) {
  const props = {
    lang: 'zh' as const,
    chatMemory: { enabled: true } as Props['chatMemory'],
    memoryDir: '/tmp/kivio/memory',
    memoryError: '',
    memorySuccess: '',
    memoryLoading: false,
    memorySavingLayer: null,
    memoryDrafts: { l1: 'L1 内容', l2: 'L2 内容' },
    memorySnapshots: { l1: 'L1 内容', l2: 'L2 内容' },
    onUpdateChatMemory: vi.fn(),
    onRefresh: vi.fn(),
    onOpenFolder: vi.fn(),
    onDraftChange: vi.fn(),
    onSaveLayer: vi.fn(),
    ...overrides,
  }
  render(<MemoryTab {...props} />)
  return props
}

describe('MemoryTab', () => {
  it('L1 / L2 各自回显自己的草稿（不串）', () => {
    renderTab()
    expect(screen.getByRole('textbox', { name: 'L1 在线记忆' })).toHaveValue('L1 内容')
    expect(screen.getByRole('textbox', { name: 'L2 长期记忆' })).toHaveValue('L2 内容')
  })

  it('只有 L1 显示字节上限', () => {
    renderTab()
    const l1 = screen.getByRole('textbox', { name: 'L1 在线记忆' }).closest<HTMLElement>('.kv-panel')!
    const l2 = screen.getByRole('textbox', { name: 'L2 长期记忆' }).closest<HTMLElement>('.kv-panel')!
    expect(within(l1).getByText(/\/ 5000 bytes/)).toBeTruthy()
    expect(within(l2).queryByText(/\/ 5000 bytes/)).toBeNull()
  })

  it('编辑 L1 时 onDraftChange 带 layer=l1', async () => {
    const props = renderTab()
    const areas = screen.getAllByRole('textbox')
    await userEvent.type(areas[0], 'x')
    expect(props.onDraftChange).toHaveBeenCalledWith('l1', expect.stringContaining('x'))
  })

  it('编辑 L2 时 onDraftChange 带 layer=l2', async () => {
    const props = renderTab()
    const areas = screen.getAllByRole('textbox')
    await userEvent.type(areas[1], 'y')
    expect(props.onDraftChange).toHaveBeenCalledWith('l2', expect.stringContaining('y'))
  })

  it('草稿与快照一致时保存按钮禁用', () => {
    renderTab()
    for (const btn of screen.getAllByRole('button', { name: '保存' })) {
      expect(btn).toBeDisabled()
    }
  })

  it('草稿改动后保存按钮可用，点击带对应 layer', async () => {
    const props = renderTab({
      memoryDrafts: { l1: 'L1 改过', l2: 'L2 内容' },
    })
    const saves = screen.getAllByRole('button', { name: '保存' })
    expect(saves[0]).not.toBeDisabled()
    await userEvent.click(saves[0])
    expect(props.onSaveLayer).toHaveBeenCalledWith('l1')
  })

  it('L1 超 5000 字节时禁用保存', () => {
    renderTab({
      memoryDrafts: { l1: 'x'.repeat(5001), l2: 'L2 内容' },
      memorySnapshots: { l1: '', l2: 'L2 内容' },
    })
    expect(screen.getAllByRole('button', { name: '保存' })[0]).toBeDisabled()
  })

  it('错误与成功提示分别渲染', () => {
    renderTab({ memoryError: '读取失败' })
    expect(screen.getByText('读取失败')).toBeTruthy()
  })

  it('保存中状态只作用于对应层', () => {
    renderTab({
      memorySavingLayer: 'l2',
      memoryDrafts: { l1: 'L1 changed', l2: 'L2 changed' },
    })
    const l1 = screen.getByRole('textbox', { name: 'L1 在线记忆' }).closest<HTMLElement>('.kv-panel')!
    const l2 = screen.getByRole('textbox', { name: 'L2 长期记忆' }).closest<HTMLElement>('.kv-panel')!
    expect(within(l1).getByRole('button', { name: '保存' })).toBeEnabled()
    expect(within(l2).getByRole('button', { name: '保存中' })).toBeDisabled()
  })
})
