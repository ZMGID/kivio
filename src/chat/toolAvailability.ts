import type { ChatToolDefinition } from '../api/tauri'

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
