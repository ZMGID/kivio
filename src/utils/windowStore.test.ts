import { describe, expect, it } from 'vitest'
import { createWindowStore } from './windowStore'

function deferred() {
  let resolve!: () => void
  let reject!: (error: Error) => void
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

describe('window-owned operation state', () => {
  it('retains pending and completed state when there are no page subscribers', async () => {
    const store = createWindowStore({ pending: false, value: '' })
    const gate = deferred()
    const leave = store.subscribe(() => {})
    const task = store.run('install', async () => {
      store.setState({ pending: true, value: '' })
      await gate.promise
      store.setState({ pending: false, value: 'installed' })
    })
    await Promise.resolve()
    leave()
    expect(store.getSnapshot()).toEqual({ pending: true, value: '' })
    gate.resolve()
    await task
    expect(store.getSnapshot()).toEqual({ pending: false, value: 'installed' })
  })

  it('coalesces same-tick starts but permits independent keys', async () => {
    const store = createWindowStore<string[]>([])
    const gate = deferred()
    const install = () => store.run('install', async () => {
      await gate.promise
      store.setState(previous => [...previous, 'install'])
    })
    const first = install()
    const duplicate = install()
    const independent = store.run('export', async () => {
      store.setState(previous => [...previous, 'export'])
    })
    expect(duplicate).toBe(first)
    await independent
    expect(store.getSnapshot()).toEqual(['export'])
    gate.resolve()
    await Promise.all([first, duplicate])
    expect(store.getSnapshot()).toEqual(['export', 'install'])
  })

  it('releases failed flights so retry can complete', async () => {
    const store = createWindowStore('draft')
    const failure = new Error('disk full')
    const rejected = store.run('save', async () => { throw failure })
    await expect(rejected).rejects.toBe(failure)
    expect(store.getSnapshot()).toBe('draft')
    await store.run('save', async () => { store.setState('saved') })
    expect(store.getSnapshot()).toBe('saved')
  })
})
