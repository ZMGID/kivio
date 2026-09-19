// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createChatNavigationController } from './chatNavigationController'
import { invalidateConversationTransition } from './conversationTransitionStore'
import type { Conversation } from './types'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

function conversation(id: string): Conversation {
  return {
    id, revision: 1, title: id, provider_id: 'test', model: 'test',
    messages: [], created_at: 1, updated_at: 1,
  }
}

function setup() {
  let current: Conversation | null = null
  const reads = new Map<string, ReturnType<typeof deferred<Conversation>>>()
  const readStarted = deferred<string>()
  const ownership = deferred<ReadonlySet<string>>()
  const shown: string[] = []
  const errors: string[] = []
  const occupyPopout = vi.fn()
  const controller = createChatNavigationController({
    currentConversation: () => current,
    currentConversationId: () => current?.id ?? null,
    listPopouts: () => ownership.promise,
    readConversation: (id) => {
      const pending = deferred<Conversation>()
      reads.set(id, pending)
      readStarted.resolve(id)
      return pending.promise
    },
    isConversationInFlight: () => false,
    focusPopout: vi.fn(),
    occupyPopout,
    prepareSelection: vi.fn(),
    showConversation: (value) => {
      current = value
      shown.push(value.id)
    },
    resetConversation: () => { current = null },
    discardConversation: (_id, error) => { errors.push(error.message) },
  })
  return { controller, ownership, reads, readStarted, shown, errors, occupyPopout, setCurrent: (value: Conversation | null) => { current = value } }
}

describe('chat navigation controller', () => {
  beforeEach(() => {
    invalidateConversationTransition()
    window.location.hash = '#chat'
  })

  it('ignores a late popout ownership result after navigating elsewhere', async () => {
    const { controller, ownership, reads, shown } = setup()
    const selecting = controller.selectConversation('a')
    controller.leaveConversation()
    window.location.hash = '#chat/settings'
    ownership.resolve(new Set())
    await selecting

    expect(reads.size).toBe(0)
    expect(shown).toEqual([])
    expect(window.location.hash).toBe('#chat/settings')
  })

  it('does not let A replace B when their popout ownership lookup settles together', async () => {
    const { controller, ownership, reads, readStarted, shown } = setup()
    const selectingA = controller.selectConversation('a')
    const selectingB = controller.selectConversation('b')
    ownership.resolve(new Set())
    expect(await readStarted.promise).toBe('b')
    reads.get('b')!.resolve(conversation('b'))
    await Promise.all([selectingA, selectingB])

    expect(reads.has('a')).toBe(false)
    expect(shown).toEqual(['b'])
    expect(window.location.hash).toBe('#chat/b')
  })

  it('does not let an old load failure erase a later route', async () => {
    const { controller, ownership, reads, readStarted, errors } = setup()
    const loading = controller.loadRouteConversation('missing-a')
    ownership.resolve(new Set())
    expect(await readStarted.promise).toBe('missing-a')
    controller.leaveConversation()
    window.location.hash = '#chat/settings'
    reads.get('missing-a')!.reject(new Error('missing'))
    await loading

    expect(errors).toEqual([])
    expect(window.location.hash).toBe('#chat/settings')
  })

  it('keeps an already open conversation when clicked again while another load is pending', async () => {
    const { controller, ownership, reads, readStarted, shown, setCurrent } = setup()
    setCurrent(conversation('a'))
    const other = controller.selectConversation('b')
    ownership.resolve(new Set())
    expect(await readStarted.promise).toBe('b')
    await controller.selectConversation('a')
    reads.get('b')!.resolve(conversation('b'))
    await other

    expect(shown).toEqual([])
    expect(reads.has('a')).toBe(false)
    expect(window.location.hash).toBe('#chat/a')
  })

  it('reconciles popout entry by replacing the main-window conversation without reading its messages', async () => {
    const { controller, reads, occupyPopout, setCurrent } = setup()
    setCurrent(conversation('a'))
    await controller.reconcilePopouts(new Set(), new Set(['a']))

    expect(occupyPopout).toHaveBeenCalledWith('a')
    expect(reads.size).toBe(0)
  })

  it('opens a different conversation by route and does not start a duplicate read', async () => {
    const { controller, reads } = setup()
    window.location.hash = '#chat/a'
    await controller.openConversation('b')

    expect(window.location.hash).toBe('#chat/b')
    expect(reads.size).toBe(0)
  })
})
