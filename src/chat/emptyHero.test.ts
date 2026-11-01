import { describe, expect, it } from 'vitest'
import { emptyHeroJab, emptyHeroJabPool, emptyHeroPinnedLine } from './emptyHero'

describe('empty hero greetings', () => {
  it('pins the selected assistant rather than a rotating greeting', () => {
    expect(emptyHeroPinnedLine({ lang: 'zh' })).toBeNull()
    expect(emptyHeroPinnedLine({ lang: 'zh', assistantName: 'assistant' })).toBe('assistant')
  })

  it('changes response pools at streak boundaries and avoids the previous response', () => {
    const mild = emptyHeroJabPool('zh', 1)
    const mid = emptyHeroJabPool('zh', 3)
    const hot = emptyHeroJabPool('zh', 6)
    const melt = emptyHeroJabPool('zh', 9)
    expect(emptyHeroJabPool('zh', 2)).toEqual(mild)
    expect(emptyHeroJabPool('zh', 5)).toEqual(mid)
    expect(emptyHeroJabPool('zh', 8)).toEqual(hot)
    expect(mild).not.toEqual(mid)
    expect(mid).not.toEqual(hot)
    expect(hot).not.toEqual(melt)
    const first = emptyHeroJab('zh', 1, null, () => 0)
    const next = emptyHeroJab('zh', 1, first, () => 0)
    expect(mild).toContain(first)
    expect(mild).toContain(next)
    expect(next).not.toBe(first)
  })
})
