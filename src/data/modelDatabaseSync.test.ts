import { describe, expect, it, vi } from 'vitest'
import { api, type ModelDatabaseSnapshot } from '../api/tauri'
import { startModelDatabaseSync, updateModelDatabase } from './modelDatabaseSync'
import { getModelDatabaseSnapshot, matchModel, resolveModelInfo } from './modelMatching'

function data(revision: number, context: number): ModelDatabaseSnapshot {
  return { revision, version: `2026-10-13.${revision}`, database: {
    _meta: { schemaVersion: 1, version: `2026-10-13.${revision}` },
    'private-model': { contextWindow: 123 },
    'sample-2.1': { displayName: 'Sample', contextWindow: context, capabilities: { vision: true } },
  } }
}

describe('desktop model database synchronization', () => {
  it('keeps an event newer than the startup response and refreshes model matching without replacing user overrides', async () => {
    let listener!: (snapshot: ModelDatabaseSnapshot) => void
    let finish!: (snapshot: ModelDatabaseSnapshot) => void
    const stop = vi.fn()
    vi.spyOn(api, 'onModelDatabaseChanged').mockImplementation(async callback => { listener = callback; return stop })
    vi.spyOn(api, 'getModelDatabase').mockImplementation(() => new Promise(resolve => { finish = resolve }))
    const starting = startModelDatabaseSync()
    await vi.waitFor(() => expect(finish).toBeDefined())
    listener(data(2, 200))
    finish(data(1, 100))
    const dispose = await starting
    expect(matchModel('sample-2-1')?.contextWindow).toBe(200)
    expect(matchModel('private-model')?.contextWindow).toBe(123)
    expect(resolveModelInfo('sample-2.1', { 'sample-2.1': { contextWindow: 999 } }).contextWindow).toBe(999)
    const update = vi.spyOn(api, 'updateModelDatabase').mockResolvedValue(data(3, 300))
    await updateModelDatabase()
    listener(data(2, 200))
    expect(matchModel('sample-2.1')?.contextWindow).toBe(300)
    expect(getModelDatabaseSnapshot().version).toBe('2026-10-13.3')
    update.mockRejectedValueOnce(new Error('offline'))
    await expect(updateModelDatabase()).rejects.toThrow('offline')
    expect(matchModel('sample-2.1')?.contextWindow).toBe(300)
    dispose()
    expect(stop).toHaveBeenCalledOnce()
    vi.restoreAllMocks()
  })
})
