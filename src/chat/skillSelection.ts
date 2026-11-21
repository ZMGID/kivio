import { refreshSettings } from '../api/settingsCache'
import { api, type SkillMeta as ApiSkillMeta } from '../api/tauri'
import { normalizeAgentRuntime } from './api'
import { deriveToolStatusHint, findUnavailableRecommendedTools, readChatToolAvailability } from './toolAvailability'
import type { Conversation, SkillMeta } from './types'
import { isTauriRuntime } from './utils'

/** 把 IPC 层的 SkillMeta 收窄成聊天页使用的形状。 */
export function normalizeSkill(skill: ApiSkillMeta): SkillMeta {
  return {
    id: skill.id,
    name: skill.name,
    description: skill.description,
    source: skill.source,
    path: skill.path ?? undefined,
    recommendedTools: skill.recommendedTools,
    disableModelInvocation: skill.disableModelInvocation,
    files: skill.files,
  }
}

export function skillRecommendedTools(skill?: SkillMeta | null): string[] {
  return skill?.recommended_tools ?? skill?.recommendedTools ?? []
}

/** One send policy for the main view and conversation popouts. */
export function resolveSendSkillId(
  enabledSkills: SkillMeta[],
  selectedSkillId: string | null,
  usesChatRuntime: boolean,
): string | null {
  if (usesChatRuntime) return null
  if (selectedSkillId && enabledSkills.some((skill) => skill.id === selectedSkillId)) return selectedSkillId
  return null
}

export async function readQueuedSendSkillPolicy(conversation: Conversation) {
  const selected = conversation.active_skill_id ?? conversation.activeSkillId
  const noSkill = { attachmentSkillId: null, disabledReason: '' }
  if (!selected || normalizeAgentRuntime(conversation).kind === 'chat' || !isTauriRuntime()) return noSkill
  try {
    const { chatTools } = await refreshSettings()
    if (chatTools?.disabledSkillIds?.includes(selected)) return noSkill
    const result = await api.chatSessionSkills(conversation.id)
    if (!result.success) return { ...noSkill, disabledReason: result.error || '读取会话技能失败，请重试。' }
    const skills = result.skills.map(normalizeSkill)
    const attachmentSkillId = resolveSendSkillId(skills, selected, false)
    const recommendedTools = skillRecommendedTools(skills.find((skill) => skill.id === attachmentSkillId))
    if (!recommendedTools.length) return { attachmentSkillId, disabledReason: '' }
    const availability = await readChatToolAvailability(chatTools)
    return {
      attachmentSkillId,
      disabledReason: deriveToolStatusHint({
        ...availability,
        effectiveSkillId: attachmentSkillId,
        recommendedTools,
        unavailableRecommendedTools: findUnavailableRecommendedTools(
          recommendedTools, availability.enabledTools, availability.toolDiscoveryPending,
        ),
      }),
    }
  } catch (error) {
    return { ...noSkill, disabledReason: error instanceof Error ? error.message : String(error) }
  }
}
