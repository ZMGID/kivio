import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ModelProvider } from '../api/tauri'
import { useProviderCatalogController } from './useProviderCatalogController'

function provider(overrides: Partial<ModelProvider> = {}): ModelProvider {
  return {
    id: 'one', name: 'One', enabled: true, baseUrl: 'https://one.test',
    apiFormat: 'openai_chat', apiKeys: ['secret'], availableModels: [], enabledModels: [],
    modelOverrides: { m1: { maxOutput: 100 } },
    ...overrides,
  } as ModelProvider
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

describe('useProviderCatalogController', () => {
  it('merges a late catalog into the current provider without undoing newer model overrides', async () => {
    const pending = deferred<{ models: string[]; capabilities: Record<string, { videoInput: boolean }> }>()
    let current = provider()
    const applied = vi.fn((updates: Partial<ModelProvider>) => { current = { ...current, ...updates } })
    const fetch = vi.fn(() => pending.promise)
    const { result } = renderHook(() => useProviderCatalogController({
      fetch,
      getProvider: () => current,
      apply: (_id, updates) => applied(updates),
    }))
    let request!: Promise<void>
    act(() => { request = result.current.fetchModels('one') })
    current = provider({ modelOverrides: { m1: { maxOutput: 200 } } })
    await act(async () => {
      pending.resolve({ models: ['m1'], capabilities: { m1: { videoInput: true } } })
      await request
    })

    expect(current.modelOverrides?.m1).toMatchObject({ maxOutput: 200, advertisedVideoInput: true })
    expect(current.availableModels).toEqual(['m1'])
    expect(result.current.fetchingProviderId).toBeNull()
  })

  it('discards a late catalog when the provider connection changed or was removed', async () => {
    const pending = deferred<{ models: string[]; capabilities: Record<string, never> }>()
    let current: ModelProvider | null = provider()
    const applied = vi.fn()
    const { result } = renderHook(() => useProviderCatalogController({
      fetch: () => pending.promise,
      getProvider: () => current,
      apply: applied,
    }))
    let request!: Promise<void>
    act(() => { request = result.current.fetchModels('one') })
    current = provider({ baseUrl: 'https://changed.test' })
    await act(async () => { pending.resolve({ models: ['stale'], capabilities: {} }); await request })
    expect(applied).not.toHaveBeenCalled()
    expect(result.current.error).toBe('')
  })

  it('does not surface a late fetch error for a deleted provider', async () => {
    const pending = deferred<{ models: string[]; capabilities: Record<string, never> }>()
    let current: ModelProvider | null = provider()
    const { result } = renderHook(() => useProviderCatalogController({
      fetch: () => pending.promise,
      getProvider: () => current,
      apply: vi.fn(),
    }))
    let request!: Promise<void>
    act(() => { request = result.current.fetchModels('one') })
    current = null
    await act(async () => { pending.reject(new Error('network failed')); await request })
    expect(result.current.error).toBe('')
  })
})
