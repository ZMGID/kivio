import type { ChatToolDefinition } from '../api/tauri'

export interface ToolStatusHintInput {
  /** 工具目录不可用的原因（空 = 可用）。 */
  toolsDisabledReason: string
  /** 已发现的工具数；发现未完成时为 null。 */
  enabledToolCount: number | null
  /** 用户是否开了任何工具来源（MCP / 原生 / Skill 运行时）。 */
  toolsRequested: boolean
  /** 当前挂载的 Skill；带 Skill 时「不支持 tools」原文直出。 */
  effectiveSkillId: string | null
  /** 当前 Skill 推荐的工具。 */
  recommendedTools: string[]
  /** 推荐里目录中找不到的那部分（见 findUnavailableRecommendedTools）。 */
  unavailableRecommendedTools: string[]
}

/**
 * 输入栏工具提示文案。优先级：目录不可用（且用户确实要工具 / Skill 要工具）→
 * 目录为空但没人要（静默）→ Skill 推荐的工具缺失。
 */
export function deriveToolStatusHint({
  toolsDisabledReason,
  enabledToolCount,
  toolsRequested,
  effectiveSkillId,
  recommendedTools,
  unavailableRecommendedTools,
}: ToolStatusHintInput): string {
  const catalogEmpty = (enabledToolCount ?? 0) === 0
  if (toolsDisabledReason && catalogEmpty && (toolsRequested || recommendedTools.length > 0)) {
    if (toolsDisabledReason.includes('不支持 tools') && effectiveSkillId) {
      return toolsDisabledReason
    }
    return recommendedTools.length > 0
      ? `当前 Skill 需要工具，但${toolsDisabledReason}`
      : toolsDisabledReason
  }
  if (toolsDisabledReason && catalogEmpty) {
    return ''
  }
  if (unavailableRecommendedTools.length > 0) {
    return `当前 Skill 推荐的工具不可用：${unavailableRecommendedTools.slice(0, 3).join(', ')}`
  }
  return ''
}

export function findUnavailableRecommendedTools(
  recommendations: string[],
  tools: ChatToolDefinition[],
  discoveryPending = false,
): string[] {
  // 未连接服务器没有工具缓存，并不表示 Skill 所需工具不可用。
  // 生成前会进行完整发现；指示器不能用部分列表提前禁止发送。
  if (discoveryPending) return []
  return recommendations.filter((recommended) => !tools.some((tool) => {
    const name = recommended.trim()
    return Boolean(name) && (
      tool.name === name ||
      tool.id === name ||
      `${tool.serverId ?? ''}:${tool.name}` === name
    )
  }))
}
