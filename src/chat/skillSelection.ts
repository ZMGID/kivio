import type { SkillMeta as ApiSkillMeta } from '../api/tauri'
import type { PendingAttachment, SkillMeta } from './types'

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

function attachmentExtension(name: string): string {
  return name.split('.').pop()?.toLowerCase() ?? ''
}

/** 文档附件对应的内置文档 Skill 名（pdf / docx / xlsx）；图片和其他类型返回 null。 */
export function documentSkillNameForAttachment(attachment: PendingAttachment): string | null {
  if (attachment.type === 'image') return null
  switch (attachmentExtension(attachment.name)) {
    case 'pdf':
      return 'pdf'
    case 'doc':
    case 'docx':
      return 'docx'
    case 'xls':
    case 'xlsx':
    case 'xlsm':
    case 'csv':
    case 'tsv':
      return 'xlsx'
    default:
      return null
  }
}

export function findEnabledSkillId(skills: SkillMeta[], skillName: string): string | null {
  const normalized = skillName.toLowerCase()
  return skills.find((skill) => (
    skill.id.toLowerCase() === normalized || skill.name.toLowerCase() === normalized
  ))?.id ?? null
}

/**
 * 附件只指向一种文档 Skill 时自动挂上它；混合多种文档类型（pdf + xlsx）不猜，交给模型。
 */
export function inferSingleAttachmentSkillId(
  attachments: PendingAttachment[],
  skills: SkillMeta[],
): string | null {
  const skillNames = Array.from(new Set(
    attachments
      .map(documentSkillNameForAttachment)
      .filter((name): name is string => Boolean(name)),
  ))
  if (skillNames.length !== 1) return null
  return findEnabledSkillId(skills, skillNames[0])
}

/** One send policy for the main view and conversation popouts. */
export function resolveSendSkillId(
  attachments: PendingAttachment[],
  enabledSkills: SkillMeta[],
  selectedSkillId: string | null,
  usesChatRuntime: boolean,
): string | null {
  if (usesChatRuntime) return null
  if (selectedSkillId && enabledSkills.some((skill) => skill.id === selectedSkillId)) return selectedSkillId
  return inferSingleAttachmentSkillId(attachments, enabledSkills)
}
