import type { SkillMeta as ApiSkillMeta } from '../api/tauri'
import type { SkillMeta } from './types'

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
