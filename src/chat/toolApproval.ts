import type { ChatToolConfirmPayload } from '../api/tauri'
import type { ApprovalAction } from './ApprovalCard'

/** 工具名 → 自然语言动词。`path` 表示操作对象是文件路径（标题里只显示文件名）。 */
const TOOL_APPROVAL_VERBS: Record<string, { verb: string; path?: boolean }> = {
  write: { verb: '写入', path: true },
  write_file: { verb: '写入', path: true },
  edit: { verb: '修改', path: true },
  edit_file: { verb: '修改', path: true },
  notebookedit: { verb: '修改', path: true },
  read: { verb: '读取', path: true },
  read_file: { verb: '读取', path: true },
  bash: { verb: '执行' },
  run_command: { verb: '执行' },
}

/**
 * 审批卡标题。后端认出操作对象（`target`）时拼「允许写入 xxx.md？」，认不出就退回工具名。
 */
export function toolApprovalTitle(payload: ChatToolConfirmPayload): string {
  const name = (payload.name || '').toLowerCase()
  const target = payload.target?.trim()
  if (name === 'exitplanmode') return '批准这份计划，开始执行？'
  if (name === 'cursor/create_plan' || name === 'create_plan') return '批准这份计划？'
  if (name === 'enterplanmode') return '让 claude 先出方案，暂不改动代码？'
  if (name === 'request_permissions' || name === 'permissions') {
    const wantsNetwork = (payload.argumentsPreview || '').includes('Network access')
    if (wantsNetwork && target) return `允许 Codex 联网并使用工作区 ${target}？`
    if (wantsNetwork) return '允许 Codex 联网？'
    if (target) return `允许 Codex 使用工作区 ${target}？`
    return '允许 Codex 使用工作区 / 执行环境？'
  }
  const spec = TOOL_APPROVAL_VERBS[name]
  if (!spec || !target) return `允许调用工具 ${payload.name}？`
  const shown = spec.path ? target.split(/[\\/]/).filter(Boolean).pop() || target : target
  return `允许${spec.verb} ${shown}？`
}

export function isClaudePlanApproval(payload: ChatToolConfirmPayload): boolean {
  return (payload.name || '').toLowerCase() === 'exitplanmode'
}

export function isCursorPlanApproval(payload: ChatToolConfirmPayload): boolean {
  const name = (payload.name || '').toLowerCase()
  return name === 'cursor/create_plan' || name === 'create_plan'
}

export function isPlanApproval(payload: ChatToolConfirmPayload): boolean {
  return isClaudePlanApproval(payload) || isCursorPlanApproval(payload)
}

export function isEnterPlanApproval(payload: ChatToolConfirmPayload): boolean {
  return (payload.name || '').toLowerCase() === 'enterplanmode'
}

export const PLAN_APPROVAL_ACTIONS: { label: string; mode: string }[] = [
  { label: '批准，逐步确认', mode: 'default' },
  { label: '批准并自动放行', mode: 'bypassPermissions' },
]

export interface ToolApprovalActionPorts {
  /** 回答后端；resolve 为 true 表示答复已被接受（不是被撤销 / 过期）。 */
  resolve: (approved: boolean, always?: boolean, permissionMode?: string | null) => Promise<boolean>
  /** 计划类批准附带一个沙盒档位：答复被接受后把它写进会话运行时。 */
  persistSandbox: (mode: string) => Promise<void> | void
}

const PRIMARY_HINT = 'Ctrl+↵'

/**
 * 审批卡按钮组。三种形态：
 * - 计划批准（exitplanmode / create_plan）：拒绝 + 每个放行档位一个按钮，批准后落沙盒档位；
 * - 进入计划模式（enterplanmode）：不用 / 总是允许 / 进入，批准后落 `plan`；
 * - 普通工具：拒绝 / 总是允许 / 允许一次。
 * 最后一个按钮永远是 primary + Ctrl+↵。
 */
export function buildToolApprovalActions(
  payload: ChatToolConfirmPayload,
  submitting: boolean,
  ports: ToolApprovalActionPorts,
): ApprovalAction[] {
  const approveThenPersist = (mode: string, always = false, permissionMode: string | null = null) => {
    void ports.resolve(true, always, permissionMode).then((accepted) => {
      if (accepted) return ports.persistSandbox(mode)
    })
  }
  if (isPlanApproval(payload)) {
    return [
      { label: '拒绝 / 让它改', disabled: submitting, onSelect: () => { void ports.resolve(false) } },
      ...PLAN_APPROVAL_ACTIONS.map((action, index) => {
        const last = index === PLAN_APPROVAL_ACTIONS.length - 1
        return {
          label: action.label,
          primary: last,
          hint: last ? PRIMARY_HINT : undefined,
          disabled: submitting,
          onSelect: () => approveThenPersist(action.mode, false, action.mode),
        }
      }),
    ]
  }
  if (isEnterPlanApproval(payload)) {
    return [
      { label: '不用，直接做', disabled: submitting, onSelect: () => { void ports.resolve(false) } },
      { label: '总是允许', disabled: submitting, onSelect: () => approveThenPersist('plan', true) },
      {
        label: '进入计划模式',
        primary: true,
        hint: PRIMARY_HINT,
        disabled: submitting,
        onSelect: () => approveThenPersist('plan'),
      },
    ]
  }
  return [
    { label: '拒绝', disabled: submitting, onSelect: () => { void ports.resolve(false) } },
    { label: '总是允许', disabled: submitting, onSelect: () => { void ports.resolve(true, true) } },
    { label: '允许一次', primary: true, hint: PRIMARY_HINT, disabled: submitting, onSelect: () => { void ports.resolve(true) } },
  ]
}
