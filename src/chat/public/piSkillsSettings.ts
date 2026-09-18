import { chatApi } from '../api'

export type { PiSkillEntry, PiSkillInventory } from '../api'

/** Settings-facing operations for Pi skill discovery and lifecycle management. */
export const piSkillsSettingsApi: Pick<
  typeof chatApi,
  | 'piSkillAddPath'
  | 'piSkillCommandsSetEnabled'
  | 'piSkillOpen'
  | 'piSkillRemove'
  | 'piSkillRemovePath'
  | 'piSkillSetEnabled'
  | 'piSkillsInventory'
  | 'piSkillsOpenDir'
> = chatApi
