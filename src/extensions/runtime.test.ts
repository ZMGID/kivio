import { beforeEach, expect, it, vi } from 'vitest'
import type { ExtensionAPI } from '../../extensions/sdk'
const fixture = vi.hoisted(() => ({ enabled: [] as string[], contexts: [] as ExtensionAPI[], veto: false, fail: false, activation: '' }))
vi.mock('../api/extensions', () => ({ extensionsApi: {
  list: async () => [...fixture.enabled],
  subscribe: async () => () => {},
  setEnabled: async (id: string, enabled: boolean) => {
    if (fixture.fail && enabled) throw new Error('disk failure')
    fixture.enabled = enabled ? [id] : []
    return [...fixture.enabled]
  },
} }))
vi.mock('../../extensions/study/index.tsx', () => ({ default: {
  Page: () => null,
  activate(api: ExtensionAPI) {
    fixture.contexts.push(api)
    api.on('activated', event => { fixture.activation = `${event.extensionId}:${event.reason}` })
    api.on('beforeDisable', () => { if (fixture.veto) return { cancel: true, reason: 'unsaved' } })
  },
} }))
beforeEach(() => {
  vi.resetModules()
  fixture.enabled = []; fixture.contexts = []; fixture.veto = false; fixture.fail = false; fixture.activation = ''
})
it('keeps disabled packages unloaded, honors the save veto, and invalidates the old context after disabling', async () => {
  const runtime = await import('./runtime')
  await runtime.initializeExtensions()
  expect(runtime.extensionPage('study')).toBeUndefined()
  expect(fixture.contexts).toHaveLength(0)
  await runtime.setExtensionEnabled('study', true)
  expect(runtime.extensionPage('study')).toBeTypeOf('function')
  expect(fixture.activation).toBe('study:enable')
  const old = fixture.contexts[0]
  const activateConversation = vi.fn().mockResolvedValue(undefined)
  const boundHost = runtime.bindExtensionHost('study', {
    onActivateConversation: activateConversation,
    onCreateConversation: vi.fn(), onSend: vi.fn(),
  })
  fixture.veto = true
  await expect(runtime.setExtensionEnabled('study', false)).rejects.toThrow('unsaved')
  expect(fixture.enabled).toEqual(['study'])
  expect(old.context.signal.aborted).toBe(false)
  fixture.veto = false
  await runtime.setExtensionEnabled('study', false)
  expect(fixture.enabled).toEqual([])
  expect(runtime.extensionPage('study')).toBeUndefined()
  expect(old.context.signal.aborted).toBe(true)
  expect(() => old.on('activated', () => {})).toThrow('失效')
  await runtime.setExtensionEnabled('study', true)
  expect(fixture.contexts[1].context.signal.aborted).toBe(false)
  await expect(boundHost.onActivateConversation(null)).rejects.toThrow('失效')
  expect(activateConversation).not.toHaveBeenCalled()
})
it('rolls back a failed activation and permits retry; restoring enabled packages sends restore context', async () => {
  let runtime = await import('./runtime')
  await runtime.initializeExtensions()
  fixture.fail = true
  await expect(runtime.setExtensionEnabled('study', true)).rejects.toThrow('disk failure')
  expect(fixture.enabled).toEqual([])
  expect(fixture.contexts[0].context.signal.aborted).toBe(true)
  expect(runtime.extensionPage('study')).toBeUndefined()
  fixture.fail = false
  await runtime.setExtensionEnabled('study', true)
  vi.resetModules()
  runtime = await import('./runtime')
  await runtime.initializeExtensions()
  expect(fixture.activation).toBe('study:restore')
  expect(runtime.extensionPage('study')).toBeTypeOf('function')
})
