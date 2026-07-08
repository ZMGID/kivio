import { describe, expect, it } from 'vitest'
import { computeSkillState } from './skillMarketState'

describe('computeSkillState', () => {
  const skill = { id: 'pdf', version: '1.0.0' }

  it('未安装 → install', () => {
    expect(computeSkillState(skill, [])).toBe('install')
    expect(computeSkillState(skill, [{ id: 'other', version: '1.0.0' }])).toBe('install')
  })

  it('已装且版本相同 → installed', () => {
    expect(computeSkillState(skill, [{ id: 'pdf', version: '1.0.0' }])).toBe('installed')
  })

  it('已装但版本不同 → update', () => {
    expect(computeSkillState(skill, [{ id: 'pdf', version: '0.9.0' }])).toBe('update')
    expect(computeSkillState(skill, [{ id: 'pdf', version: '2.0.0' }])).toBe('update')
  })
})
