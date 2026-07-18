// skillMarket 数据层自检：列表归一、owner 消歧、下载链构造。
import { describe, expect, it } from 'vitest'
import {
  buildClawHubDownloadUrl,
  listClawHubSkills,
  normalizeClawHubSkillCard,
  resolveClawHubSkillOwner,
  selectClawHubOwnerCandidate,
  type ClawHubSkillCard,
} from './skillMarket'

function fakeFetch(payload: unknown): typeof fetch {
  return (async () =>
    ({
      ok: true,
      status: 200,
      text: async () => JSON.stringify(payload),
    }) as Response) as unknown as typeof fetch
}

const baseCard = (over: Partial<ClawHubSkillCard>): ClawHubSkillCard => ({
  slug: 'x',
  displayName: 'x',
  summary: '',
  latestVersion: null,
  downloads: 0,
  stars: 0,
  installsCurrent: 0,
  updatedAt: null,
  ownerHandle: null,
  webUrl: null,
  downloadUrl: null,
  ...over,
})

describe('skillMarket', () => {
  it('normalizes list card with nested stats', async () => {
    const res = await listClawHubSkills({
      sort: 'downloads',
      fetchImpl: fakeFetch({
        items: [
          {
            slug: 'pdf',
            displayName: 'PDF Skill',
            summary: 'work with pdf',
            tags: { latest: '1.2.0' },
            stats: { downloads: 999, stars: 12, installs: 30 },
            owner: { handle: 'alice' },
          },
        ],
        nextCursor: 'c2',
      }),
    })
    expect(res.nextCursor).toBe('c2')
    const card = res.items[0]
    expect(card.downloads).toBe(999)
    expect(card.stars).toBe(12)
    expect(card.installsCurrent).toBe(30)
    expect(card.latestVersion).toBe('1.2.0')
    expect(card.ownerHandle).toBe('alice')
    expect(card.downloadUrl).toContain('ownerHandle=alice')
  })

  it('buildClawHubDownloadUrl carries slug/tag/owner', () => {
    const url = buildClawHubDownloadUrl('my-skill', 'bob')
    expect(url).toContain('slug=my-skill')
    expect(url).toContain('tag=latest')
    expect(url).toContain('ownerHandle=bob')
  })

  it('disambiguates owner among same-slug candidates by downloads', () => {
    const skill = baseCard({ slug: 's', downloads: 500 })
    const candidates = [
      baseCard({ slug: 's', ownerHandle: 'a', downloads: 100 }),
      baseCard({ slug: 's', ownerHandle: 'b', downloads: 500 }),
    ]
    const picked = selectClawHubOwnerCandidate(skill, candidates)
    expect(picked?.ownerHandle).toBe('b')
  })

  it('resolveClawHubSkillOwner uses search when card lacks owner', async () => {
    const skill = baseCard({ slug: 'weather', downloads: 42, summary: 'wx' })
    const resolved = await resolveClawHubSkillOwner(
      skill,
      fakeFetch({
        results: [
          { slug: 'weather', owner: { handle: 'carol' }, downloads: 42, summary: 'wx' },
          { slug: 'weather', owner: { handle: 'dave' }, downloads: 7 },
        ],
      }),
    )
    expect(resolved.ownerHandle).toBe('carol')
    expect(resolved.downloadUrl).toContain('ownerHandle=carol')
  })

  it('throws when owner cannot be disambiguated', async () => {
    const skill = baseCard({ slug: 'dup' })
    await expect(
      resolveClawHubSkillOwner(
        skill,
        fakeFetch({
          results: [
            { slug: 'dup', owner: { handle: 'a' } },
            { slug: 'dup', owner: { handle: 'b' } },
          ],
        }),
      ),
    ).rejects.toThrow()
  })

  it('normalizeClawHubSkillCard rejects entries without slug', () => {
    expect(normalizeClawHubSkillCard({ displayName: 'no slug' })).toBeNull()
  })
})
