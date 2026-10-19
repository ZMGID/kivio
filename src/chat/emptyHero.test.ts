import { describe, expect, it } from 'vitest'
import { emptyHeroGreetings, emptyHeroJab, emptyHeroJabPool, emptyHeroPinnedLine } from './emptyHero'

describe('empty hero greetings', () => {
  it('问候有多条，钉住文案不走轮换池', () => {
    expect(emptyHeroGreetings('zh').length).toBeGreaterThan(1)
    expect(emptyHeroGreetings('en').length).toBe(emptyHeroGreetings('zh').length)
    expect(emptyHeroPinnedLine({ lang: 'zh' })).toBeNull()
    expect(emptyHeroPinnedLine({ lang: 'zh', assistantName: '翻译官' })).toBe('翻译官')
  })

  it('点击回应按连点档位走，避开刚说过的', () => {
    expect(emptyHeroJabPool('zh', 1)[0]).toBe('嗯？我在')
    expect(emptyHeroJabPool('zh', 3)[0]).toBe('有点痒')
    expect(emptyHeroJabPool('zh', 6)[0]).toBe('脸有点热了')
    expect(emptyHeroJabPool('zh', 9)[0]).toBe('晕乎乎的')
    expect(emptyHeroJabPool('en', 6).length).toBe(emptyHeroJabPool('zh', 6).length)
    expect(emptyHeroJab('zh', 1, null, () => 0)).toBe('嗯？我在')
    expect(emptyHeroJab('zh', 1, '嗯？我在', () => 0)).toBe('收到一枚轻戳')
    expect(emptyHeroJab('en', 6, null, () => 0)).toBe('Getting a little warm.')
  })
})
