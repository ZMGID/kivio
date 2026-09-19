// Chat 自动化图编辑器的节点分类与分支端口语义。
import type { AutomationNodeType, FlowNodeData } from '../../api/automationContracts'

export function isTriggerType(type: string): type is AutomationNodeType {
  return type.startsWith('trigger.')
}

export function isActionType(type: string): type is AutomationNodeType {
  return type.startsWith('action.')
}

export function isStepType(type: string): boolean {
  return type.startsWith('action.') || type.startsWith('logic.')
}

export function isAttachmentType(type: string): boolean {
  return type.startsWith('agent.')
}

export function isIfType(type: string): boolean {
  return type === 'logic.if'
}

export function isSwitchType(type: string): boolean {
  return type === 'logic.switch'
}

export function branchHandles(type: string, data?: FlowNodeData): string[] | null {
  if (isIfType(type)) return ['true', 'false']
  if (isSwitchType(type)) {
    const ids = (data?.switch?.cases ?? [])
      .map((item) => item.id.trim())
      .filter(Boolean)
    return [...ids, 'default']
  }
  return null
}
