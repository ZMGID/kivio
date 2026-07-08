import type { MarketInstalledInfo, MarketSkill } from '../api/tauri'

export type SkillMarketState = 'install' | 'installed' | 'update'

/** 由市场技能 + 本地已装清单算三态：未装 / 已装(版本同) / 可更新(版本不同)。 */
export function computeSkillState(
  skill: Pick<MarketSkill, 'id' | 'version'>,
  installed: MarketInstalledInfo[],
): SkillMarketState {
  const hit = installed.find((i) => i.id === skill.id)
  if (!hit) return 'install'
  // 版本字符串不同即视为可更新（不做语义版本比较：索引 version 是权威值）。
  return hit.version !== skill.version ? 'update' : 'installed'
}
