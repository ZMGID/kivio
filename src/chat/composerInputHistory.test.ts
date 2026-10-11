import { describe, expect, it } from 'vitest'
import { composerInputHistory } from './composerInputHistory'
import type { ChatMessage } from './types'

function user(id: string, content: string, peer_message?: ChatMessage['peer_message']): ChatMessage {
  return { id, role: 'user', content, timestamp: 1, peer_message }
}

describe('composerInputHistory', () => {
  it('keeps typed user turns and drops colleague deliveries', () => {
    const messages: ChatMessage[] = [
      user('mine', 'actual question'),
      user('peer', 'from Ada', { deliveryId: 'dlv-1', exchangeId: 'ex-1', from: { kind: 'bot', id: 'ada', name: 'Ada' }, toBotId: 'me', toName: 'Me', kind: 'peer' }),
      { id: 'answer', role: 'assistant', content: 'reply', timestamp: 2 },
    ]
    expect(composerInputHistory(messages)).toEqual(['actual question'])
  })

  it('does not treat a marker in the body as a colleague delivery', () => {
    const content = '[kivio-peer dlv_1]\nhello'
    expect(composerInputHistory([user('typed', content)])).toEqual([content])
  })
})
