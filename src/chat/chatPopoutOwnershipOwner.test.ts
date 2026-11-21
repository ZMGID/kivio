import { describe, expect, it, vi } from 'vitest'
import { createChatPopoutOwnershipOwner } from './chatPopoutOwnershipOwner'

describe('chat popout ownership owner', () => {
  it('does not let an A ownership query arriving late replace the newer B event', async () => {
    let resolveList!: (ids: string[]) => void
    const listConversationPopouts = vi.fn(() => new Promise<string[]>((resolve) => { resolveList = resolve }))
    const owner = createChatPopoutOwnershipOwner({
      listConversationPopouts, openConversationPopout: vi.fn(), closeConversationPopout: vi.fn(),
    })
    const pending = owner.list()
    const changed = owner.changed(['b'])
    expect([...changed.entered]).toEqual(['b'])
    resolveList(['a'])
    expect([...(await pending)]).toEqual(['b'])
    expect([...owner.membership()]).toEqual(['b'])
    expect(listConversationPopouts).toHaveBeenCalledTimes(1)
  })

  it('re-fetches membership after a sidebar refresh instead of trusting an old cache', async () => {
    const listConversationPopouts = vi.fn()
      .mockResolvedValueOnce(['a'])
      .mockResolvedValueOnce(['b'])
    const owner = createChatPopoutOwnershipOwner({
      listConversationPopouts, openConversationPopout: vi.fn(), closeConversationPopout: vi.fn(),
    })
    expect([...(await owner.list())]).toEqual(['a'])
    expect([...(await owner.list())]).toEqual(['a'])
    expect(listConversationPopouts).toHaveBeenCalledTimes(1)
    const change = await owner.refresh()
    expect([...change.previous]).toEqual(['a'])
    expect([...change.next]).toEqual(['b'])
    expect([...change.entered]).toEqual(['b'])
    expect([...change.exited]).toEqual(['a'])
    expect(listConversationPopouts).toHaveBeenCalledTimes(2)
  })

  it('does not resurrect a popout when an open response arrives after a newer close event', async () => {
    let finishOpen!: () => void
    const owner = createChatPopoutOwnershipOwner({
      listConversationPopouts: vi.fn().mockResolvedValue([]),
      openConversationPopout: vi.fn(() => new Promise<void>((resolve) => { finishOpen = resolve })),
      closeConversationPopout: vi.fn(),
    })
    const opening = owner.open('a')
    owner.changed(['a'])
    owner.changed([])
    finishOpen()
    const result = await opening
    expect([...result.next]).toEqual([])
    expect([...owner.membership()]).toEqual([])
  })

  it('does not change ownership when an open or close request fails', async () => {
    const owner = createChatPopoutOwnershipOwner({
      listConversationPopouts: vi.fn(),
      openConversationPopout: vi.fn().mockRejectedValue(new Error('open denied')),
      closeConversationPopout: vi.fn().mockRejectedValue(new Error('close denied')),
    })
    await expect(owner.open('a')).rejects.toThrow('open denied')
    expect(owner.owns('a')).toBe(false)
    owner.changed(['a'])
    await expect(owner.close('a')).rejects.toThrow('close denied')
    expect(owner.owns('a')).toBe(true)
  })
})
