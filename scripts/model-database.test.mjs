import { test } from 'node:test'
import assert from 'node:assert/strict'
import { validateDatabase, compareVersions } from './model-database.mjs'

const catalog = () => ({ _meta: { schemaVersion: 1, version: '2026-10-13.1' }, sample: {
  displayName: 'Sample', contextWindow: 200000, capabilities: { vision: true }, pricing: { input: 1 }, reasoningEfforts: ['minimal', 'high'],
} })
test('publication gate rejects incompatible, incomplete and malformed model data', () => {
  assert.equal(validateDatabase(JSON.stringify(catalog())).sample.contextWindow, 200000)
  for (const change of [
    d => { d._meta.schemaVersion = 2 },
    d => { d._meta.version = 'latest' },
    d => { delete d.sample },
    d => { d.sample.capabilities.vision = 'true' },
    d => { d.sample.contextWindow = -1 },
    d => { d.sample.pricing.input = '1' },
    d => { d.sample.reasoningEfforts = ['invented'] },
  ]) {
    const data = catalog()
    change(data)
    assert.throws(() => validateDatabase(JSON.stringify(data)))
  }
})
test('publication order handles same-day revisions and calendar boundaries', () => {
  assert.equal(compareVersions('2026-10-13.10', '2026-10-13.9'), 1)
  assert.equal(compareVersions('2026-10-13.1', '2026-10-13.1'), 0)
  assert.equal(compareVersions('2026-10-13.10', '2026-10-14.1'), -1)
})
