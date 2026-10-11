import { describe, expect, it } from 'vitest'
import { privateRunCards } from './privateRunAttention'

describe('privateRunCards', () => {
  const members = [
    { botId: 'asst_ada', name: 'Ada', conversationId: 'conv_bot_member_asst_ada@room' },
    { botId: 'asst_bea', name: 'Bea', conversationId: 'conv_bot_bea' },
  ]

  it('keeps only dedicated member threads and drops prompt text', () => {
    const secret = 'sk-live-do-not-share'
    const cards = privateRunCards(members, [
      { conversationId: 'conv_bot_member_asst_ada@room', kind: 'question' },
      { conversationId: 'conv_bot_bea', kind: 'approval' },
      { conversationId: 'conv_bot_member_other@room', kind: 'approval' },
    ])
    expect(cards).toEqual([
      {
        botId: 'asst_ada',
        name: 'Ada',
        conversationId: 'conv_bot_member_asst_ada@room',
        kind: 'question',
      },
    ])
    expect(JSON.stringify(cards)).not.toContain(secret)
    expect(JSON.stringify(cards)).not.toContain('shell')
  })
})
