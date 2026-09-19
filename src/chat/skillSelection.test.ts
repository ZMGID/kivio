import { describe, expect, it } from 'vitest'
import type { PendingAttachment, SkillMeta } from './types'
import {
  documentSkillNameForAttachment,
  findEnabledSkillId,
  inferSingleAttachmentSkillId,
  normalizeSkill,
  resolveSendSkillId,
  skillRecommendedTools,
} from './skillSelection'

const skill = (id: string, name = id): SkillMeta => ({
  id, name, description: '', source: 'builtin',
} as SkillMeta)

const file = (name: string, type: PendingAttachment['type'] = 'file'): PendingAttachment => ({
  id: name, name, type, path: `/tmp/${name}`,
} as PendingAttachment)

describe('documentSkillNameForAttachment', () => {
  it('maps document extensions to the bundled document skills', () => {
    expect(documentSkillNameForAttachment(file('report.PDF'))).toBe('pdf')
    expect(documentSkillNameForAttachment(file('memo.doc'))).toBe('docx')
    expect(documentSkillNameForAttachment(file('memo.docx'))).toBe('docx')
    for (const ext of ['xls', 'xlsx', 'xlsm', 'csv', 'tsv']) {
      expect(documentSkillNameForAttachment(file(`data.${ext}`))).toBe('xlsx')
    }
  })

  it('never suggests a document skill for images or unknown files', () => {
    expect(documentSkillNameForAttachment(file('shot.pdf', 'image'))).toBeNull()
    expect(documentSkillNameForAttachment(file('notes.txt'))).toBeNull()
    expect(documentSkillNameForAttachment(file('noext'))).toBeNull()
  })
})

describe('resolveSendSkillId', () => {
  const skills = [skill('pdf-skill', 'pdf'), skill('xlsx')]

  it('uses an enabled selection before attachment inference', () => {
    expect(resolveSendSkillId([file('sheet.xlsx')], skills, 'pdf-skill', false)).toBe('pdf-skill')
  })

  it('ignores disabled selections and infers a single document skill', () => {
    expect(resolveSendSkillId([file('report.pdf')], skills, 'disabled', false)).toBe('pdf-skill')
  })

  it('never attaches a skill to the chat runtime', () => {
    expect(resolveSendSkillId([file('report.pdf')], skills, 'pdf-skill', true)).toBeNull()
  })
})

describe('inferSingleAttachmentSkillId', () => {
  const skills = [skill('pdf-skill', 'pdf'), skill('xlsx')]

  it('picks the enabled skill when all attachments agree on one document kind', () => {
    expect(inferSingleAttachmentSkillId([file('a.pdf'), file('b.pdf'), file('c.png', 'image')], skills))
      .toBe('pdf-skill')
  })

  it('refuses to guess when attachments span multiple document kinds', () => {
    expect(inferSingleAttachmentSkillId([file('a.pdf'), file('b.xlsx')], skills)).toBeNull()
  })

  it('returns null when the matching skill is not enabled', () => {
    expect(inferSingleAttachmentSkillId([file('a.docx')], skills)).toBeNull()
  })

  it('matches by id or name, case-insensitively', () => {
    expect(findEnabledSkillId(skills, 'PDF')).toBe('pdf-skill')
    expect(findEnabledSkillId(skills, 'XLSX')).toBe('xlsx')
    expect(findEnabledSkillId(skills, 'docx')).toBeNull()
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
