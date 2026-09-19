import { describe, expect, it, vi } from 'vitest'
import {
  buildToolApprovalActions,
  isClaudePlanApproval,
  isCursorPlanApproval,
  isPlanApproval,
  toolApprovalTitle,
} from './toolApproval'

function payload(name: string) {
  return { name, conversationId: 'c', runId: 'r', toolCallId: 't', source: 'cli' }
}

describe('toolApproval plan cards', () => {
  it('keeps claude ExitPlanMode as the Claude plan card', () => {
    const item = payload('ExitPlanMode')
    expect(isClaudePlanApproval(item)).toBe(true)
    expect(isCursorPlanApproval(item)).toBe(false)
    expect(isPlanApproval(item)).toBe(true)
    expect(toolApprovalTitle(item)).toBe('批准这份计划，开始执行？')
  })

  it('treats cursor/create_plan as its own plan card, not Claude auto-allow', () => {
    const item = payload('cursor/create_plan')
    expect(isClaudePlanApproval(item)).toBe(false)
    expect(isCursorPlanApproval(item)).toBe(true)
    expect(isPlanApproval(item)).toBe(true)
    expect(toolApprovalTitle(item)).toBe('批准这份计划？')
  })
})

describe('buildToolApprovalActions', () => {
  const ports = () => ({
    resolve: vi.fn().mockResolvedValue(true),
    persistSandbox: vi.fn(),
  })

  it('plan approval: reject + one button per mode, last is primary + Ctrl+↵', async () => {
    const p = ports()
    const actions = buildToolApprovalActions(payload('exitplanmode'), false, p)
    expect(actions.map((a) => a.label)).toEqual(['拒绝 / 让它改', '批准，逐步确认', '批准并自动放行'])
    expect(actions[0].primary).toBeUndefined()
    expect(actions[1].primary).toBe(false)
    expect(actions[2]).toMatchObject({ primary: true, hint: 'Ctrl+↵' })
    expect(actions.every((a) => a.disabled === false)).toBe(true)

    actions[0].onSelect()
    expect(p.resolve).toHaveBeenCalledWith(false)
    expect(p.persistSandbox).not.toHaveBeenCalled()

    actions[2].onSelect()
    await Promise.resolve()
    expect(p.resolve).toHaveBeenCalledWith(true, false, 'bypassPermissions')
    expect(p.persistSandbox).toHaveBeenCalledWith('bypassPermissions')
  })

  it('persists the sandbox only after the plan approval is accepted', async () => {
    const p = ports()
    p.resolve.mockResolvedValue(false)
    const actions = buildToolApprovalActions(payload('create_plan'), true, p)
    expect(actions.every((a) => a.disabled === true)).toBe(true)
    actions[1].onSelect()
    await Promise.resolve()
    expect(p.resolve).toHaveBeenCalledWith(true, false, 'default')
    expect(p.persistSandbox).not.toHaveBeenCalled()
  })

  it('enterplanmode: 不用 / 总是允许 / 进入计划模式', async () => {
    const p = ports()
    const actions = buildToolApprovalActions(payload('EnterPlanMode'), false, p)
    expect(actions.map((a) => a.label)).toEqual(['不用，直接做', '总是允许', '进入计划模式'])
    expect(actions[2]).toMatchObject({ primary: true, hint: 'Ctrl+↵' })

    actions[1].onSelect()
    await Promise.resolve()
    expect(p.resolve).toHaveBeenCalledWith(true, true, null)
    expect(p.persistSandbox).toHaveBeenCalledWith('plan')

    p.resolve.mockClear()
    p.persistSandbox.mockClear()
    actions[2].onSelect()
    await Promise.resolve()
    expect(p.resolve).toHaveBeenCalledWith(true, false, null)
    expect(p.persistSandbox).toHaveBeenCalledWith('plan')
  })

  it('ordinary tool: 拒绝 / 总是允许 / 允许一次, no sandbox persist', async () => {
    const p = ports()
    const actions = buildToolApprovalActions(payload('write_file'), false, p)
    expect(actions.map((a) => a.label)).toEqual(['拒绝', '总是允许', '允许一次'])
    expect(actions[2]).toMatchObject({ primary: true, hint: 'Ctrl+↵' })

    actions[1].onSelect()
    await Promise.resolve()
    expect(p.resolve).toHaveBeenCalledWith(true, true)
    expect(p.persistSandbox).not.toHaveBeenCalled()

    actions[2].onSelect()
    await Promise.resolve()
    expect(p.resolve).toHaveBeenCalledWith(true)
  })
})
