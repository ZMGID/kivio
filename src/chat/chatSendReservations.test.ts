import { describe, expect, it } from 'vitest'
import { createChatSendReservations } from './chatSendReservations'

describe('chat send reservations', () => {
  it('claims the blank composer before asynchronous conversation creation', () => {
    const sends = createChatSendReservations()
    const first = sends.claim(null)
    expect(first).not.toBeNull()
    expect(sends.claim(null)).toBeNull()

    expect(first!.bind('created-a')).toBe(true)
    expect(sends.claim(null)).not.toBeNull()
    expect(sends.claim('created-a')).toBeNull()
    first!.release()
    expect(sends.claim('created-a')).not.toBeNull()
  })

  it('lets different conversations send in parallel and releases after failure', () => {
    const sends = createChatSendReservations()
    const a = sends.claim('a')!
    const b = sends.claim('b')!
    expect(a).not.toBeNull()
    expect(b).not.toBeNull()
    expect(sends.claim('a')).toBeNull()
    a.release()
    a.release()
    expect(sends.claim('a')).not.toBeNull()
    expect(sends.claim('b')).toBeNull()
  })

  it('does not steal another transaction when a draft is bound to an occupied id', () => {
    const sends = createChatSendReservations()
    const existing = sends.claim('a')!
    const draft = sends.claim(null)!
    expect(draft.bind('a')).toBe(false)
    expect(sends.claim('a')).toBeNull()
    draft.release()
    existing.release()
    expect(sends.claim('a')).not.toBeNull()
  })
})
