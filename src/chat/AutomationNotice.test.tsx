import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { AutomationNotice } from './AutomationNotice'
import type { ToolCallRecord } from './types'

vi.mock('./chatRoutes', () => ({
  automationHash: (id: string) => `#chat/automations/${id}`,
  setHash: vi.fn(),
}))
import { setHash } from './chatRoutes'

function call(toolName: string, data: Partial<ToolCallRecord> = {}): ToolCallRecord {
  return { id: 'tool-1', toolName, status: 'completed', ...data }
}

describe('AutomationNotice', () => {
  it.each([
    ['automation_upsert', { automation: { id: 'a1', name: '早间简报' }, created: true }, '已创建工作流'],
    ['automation_upsert', { automation: { id: 'a1', name: '早间简报' }, created: false }, '已更新工作流'],
    ['automation_upsert', { automation: { id: 'a1', name: '早间简报' } }, '已保存工作流'],
    ['automation_set_enabled', { automation: { id: 'a1', name: '早间简报', enabled: false } }, '已暂停工作流'],
    ['automation_set_enabled', { automation: { id: 'a1', name: '早间简报', enabled: true } }, '已启用工作流'],
  ])('renders %s mutation receipts with their persisted name and opens the task', (toolName, payload, label) => {
    render(<AutomationNotice toolCall={call(toolName, { structured_content: payload, arguments: { automation: { name: '旧名字' } } })} />)
    fireEvent.click(screen.getByRole('button', { name: `${label} · 早间简报` }))
    expect(setHash).toHaveBeenLastCalledWith('#chat/automations/a1')
    expect(screen.queryByText('旧名字')).not.toBeInTheDocument()
  })

  it.each([
    ['schedule_create', { created: { id: 's1', name: '每日回顾' } }, {}, '已创建例行任务'],
    ['schedule_update', { updated: { id: 's1', name: '每日回顾', enabled: true } }, {}, '已更新例行任务'],
    ['schedule_update', { updated: { id: 's1', name: '每日回顾', enabled: false } }, { enabled: false }, '已暂停例行任务'],
    ['schedule_delete', { deleted: 's1' }, {}, '已删除例行任务'],
  ])('uses the scheduled task result without rendering prompt JSON for %s', (toolName, payload, args, label) => {
    render(<AutomationNotice toolCall={call(toolName, { result: JSON.stringify(payload), arguments: args })} />)
    expect(screen.getByText(label)).toBeVisible()
    expect(screen.queryByText(/"created"|"updated"|"deleted"/)).not.toBeInTheDocument()
    if (toolName === 'schedule_delete') expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it.each(['automation_list', 'automation_get', 'automation_runs', 'schedule_list'])('hides successful %s queries but keeps progress and failure visible', (toolName) => {
    const { container, rerender } = render(<AutomationNotice toolCall={call(toolName)} />)
    expect(container).toBeEmptyDOMElement()
    rerender(<AutomationNotice toolCall={call(toolName, { status: 'running' })} />)
    expect(screen.getByText(toolName.startsWith('schedule_') ? '正在读取例行任务' : '正在读取工作流')).toBeVisible()
    rerender(<AutomationNotice toolCall={call(toolName, { status: 'error', error: '{"token":"secret","error":"权限不足"}' })} />)
    expect(screen.getByRole('alert')).toHaveTextContent(toolName.startsWith('schedule_') ? '例行任务操作失败' : '工作流操作失败')
    expect(screen.queryByText(/secret/)).not.toBeInTheDocument()
  })

  it('reports validation without claiming that a dry-run candidate was saved', () => {
    render(<AutomationNotice toolCall={call('automation_upsert', {
      arguments: { dry_run: true },
      structured_content: { dryRun: true, automation: { id: 'a1', name: '候选任务' } },
    })} />)
    expect(screen.getByText('工作流校验通过')).toBeVisible()
    expect(screen.queryByText('已保存工作流')).not.toBeInTheDocument()
  })
})
