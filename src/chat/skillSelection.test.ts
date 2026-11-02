import { describe, expect, it } from 'vitest'
import type { SkillMeta } from './types'
import {
  normalizeSkill,
  resolveSendSkillId,
  skillRecommendedTools,
} from './skillSelection'

const skill = (id: string, name = id): SkillMeta => ({
  id, name, description: '', source: 'builtin',
} as SkillMeta)

describe('resolveSendSkillId', () => {
  const skills = [skill('review-notes'), skill('pdf')]

  it('uses the explicitly selected enabled skill regardless of its name', () => {
    expect(resolveSendSkillId(skills, 'review-notes', false)).toBe('review-notes')
  })

  it('does not replace an unavailable selection with a skill of a conventional name', () => {
    expect(resolveSendSkillId(skills, 'disabled', false)).toBeNull()
    expect(resolveSendSkillId(skills, null, false)).toBeNull()
  })

  it('never attaches a skill to the chat runtime', () => {
    expect(resolveSendSkillId(skills, 'review-notes', true)).toBeNull()
  })
})

describe('skill meta helpers', () => {
  it('reads recommended tools from either field spelling', () => {
    expect(skillRecommendedTools({ ...skill('a'), recommended_tools: ['x'] })).toEqual(['x'])
    expect(skillRecommendedTools({ ...skill('a'), recommendedTools: ['y'] })).toEqual(['y'])
    expect(skillRecommendedTools(null)).toEqual([])
  })

  it('normalizes the IPC skill shape and drops null paths', () => {
    const normalized = normalizeSkill({
      id: 's', name: 'S', description: 'd', source: 'user', path: null,
      recommendedTools: ['t'], disableModelInvocation: true, files: ['SKILL.md'],
    } as never)
    expect(normalized).toEqual({
      id: 's', name: 'S', description: 'd', source: 'user', path: undefined,
      recommendedTools: ['t'], disableModelInvocation: true, files: ['SKILL.md'],
    })
  })
})
