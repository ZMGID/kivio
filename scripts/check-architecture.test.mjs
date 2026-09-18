import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, test } from 'node:test'
import {
  collectCrossDomainEdges,
  isPublicInterfaceEdge,
  newViolations,
} from './check-architecture.mjs'

const fixtures = []
afterEach(() => {
  for (const fixture of fixtures.splice(0)) fs.rmSync(fixture, { recursive: true, force: true })
})

function fixture(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'kivio-architecture-'))
  fixtures.push(root)
  fs.writeFileSync(path.join(root, 'tsconfig.json'), JSON.stringify({
    compilerOptions: { moduleResolution: 'bundler', baseUrl: '.', paths: { '@/*': ['src/*'] } },
    include: ['src'],
  }))
  for (const [name, contents] of Object.entries(files)) {
    const target = path.join(root, name)
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(target, contents)
  }
  return root
}

test('resolves relative, aliased, re-exported and dynamic literal imports', () => {
  const root = fixture({
    'src/chat/relative.ts': "import '../settings/a'",
    'src/chat/alias.ts': "export { a } from '@/settings/a'",
    'src/chat/dynamic.ts': "void import('../settings/a')",
    'src/settings/a.ts': 'export const a = 1',
  })
  assert.deepEqual(collectCrossDomainEdges(root), [
    { source: 'src/chat/alias.ts', target: 'src/settings/a.ts' },
    { source: 'src/chat/dynamic.ts', target: 'src/settings/a.ts' },
    { source: 'src/chat/relative.ts', target: 'src/settings/a.ts' },
  ])
})

test('allows a feature to depend only on another feature public interface', () => {
  const root = fixture({
    'src/chat/allowed.ts': "import '../settings/public/i18n'",
    'src/chat/blocked.ts': "import '../settings/internal'",
    'src/settings/public/i18n.ts': "export { value } from '../internal'",
    'src/settings/internal.ts': 'export const value = 1',
  })

  const edges = collectCrossDomainEdges(root)
  assert.equal(edges.some(isPublicInterfaceEdge), true)
  assert.deepEqual(newViolations(root, { existingViolations: [] }), [{
    kind: 'deep_feature_import',
    source: 'src/chat/blocked.ts',
    target: 'src/settings/internal.ts',
  }])
})

test('detects a cross-feature cycle even when both imports use public interfaces', () => {
  const root = fixture({
    'src/chat/public/a.ts': "export { b } from '../../settings/public/b'",
    'src/settings/public/b.ts': "export { a } from '../../chat/public/a'",
  })

  assert.deepEqual(newViolations(root, { existingViolations: [] }), [
    {
      kind: 'cross_feature_cycle',
      source: 'src/chat/public/a.ts',
      target: 'src/chat/public/a.ts',
      cycle: ['src/chat/public/a.ts', 'src/settings/public/b.ts'],
    },
    {
      kind: 'public_reverse_dependency',
      source: 'src/chat/public/a.ts',
      target: 'src/settings/public/b.ts',
    },
    {
      kind: 'public_reverse_dependency',
      source: 'src/settings/public/b.ts',
      target: 'src/chat/public/a.ts',
    },
  ])
})

test('does not let a non-public barrel hide a deep feature import', () => {
  const root = fixture({
    'src/chat/consumer.ts': "import '../settings/index'",
    'src/settings/index.ts': "export { value } from './internal'",
    'src/settings/internal.ts': 'export const value = 1',
  })

  assert.deepEqual(newViolations(root, { existingViolations: [] }), [{
    kind: 'deep_feature_import',
    source: 'src/chat/consumer.ts',
    target: 'src/settings/index.ts',
  }])
})

test('forbids foundation modules and feature public interfaces from depending on a feature', () => {
  const root = fixture({
    'src/utils/shared.ts': "import '../chat/public/model'",
    'src/chat/public/model.ts': "export { settings } from '../../settings/public/settings'",
    'src/settings/public/settings.ts': 'export const settings = 1',
  })

  assert.deepEqual(newViolations(root, { existingViolations: [] }), [
    {
      kind: 'public_reverse_dependency',
      source: 'src/chat/public/model.ts',
      target: 'src/settings/public/settings.ts',
    },
    {
      kind: 'shared_to_feature',
      source: 'src/utils/shared.ts',
      target: 'src/chat/public/model.ts',
    },
  ])
})

test('forbids shared UI components from depending on a feature', () => {
  const root = fixture({
    'src/components/shared.tsx': "import '../chat/public/model'",
    'src/chat/public/model.ts': 'export const model = 1',
  })

  assert.deepEqual(newViolations(root, { existingViolations: [] }), [{
    kind: 'shared_to_feature',
    source: 'src/components/shared.tsx',
    target: 'src/chat/public/model.ts',
  }])
})

test('detects cross-feature cycles that pass through an adapter', () => {
  const root = fixture({
    'src/chat/public/a.ts': "export { bridge } from '../../api/bridge'",
    'src/api/bridge.ts': "export { b as bridge } from '../settings/public/b'",
    'src/settings/public/b.ts': "export { a as b } from '../../chat/public/a'",
  })

  assert.deepEqual(newViolations(root, { existingViolations: [] }), [
    {
      kind: 'cross_feature_cycle',
      source: 'src/api/bridge.ts',
      target: 'src/api/bridge.ts',
      cycle: ['src/api/bridge.ts', 'src/chat/public/a.ts', 'src/settings/public/b.ts'],
    },
    {
      kind: 'public_reverse_dependency',
      source: 'src/settings/public/b.ts',
      target: 'src/chat/public/a.ts',
    },
  ])
})

test('allows only an explicitly registered composition root to mount feature implementations', () => {
  const root = fixture({
    'src/App.tsx': "import './chat/Chat'",
    'src/themeColors.ts': "import './settings/internal'",
    'src/chat/Chat.tsx': 'export const Chat = 1',
    'src/settings/internal.ts': 'export const settings = 1',
  })

  assert.deepEqual(newViolations(root, { existingViolations: [] }), [{
    kind: 'shared_to_feature',
    source: 'src/themeColors.ts',
    target: 'src/settings/internal.ts',
  }])
})

test('an exact baseline permits only the recorded violation kind and edge', () => {
  const root = fixture({
    'src/chat/a.ts': "import '../settings/b'",
    'src/onboarding/a.ts': "import '../settings/b'",
    'src/settings/b.ts': 'export const b = 1',
  })

  assert.deepEqual(newViolations(root, {
    existingViolations: [{
      kind: 'deep_feature_import',
      source: 'src/chat/a.ts',
      target: 'src/settings/b.ts',
    }],
  }), [{
    kind: 'deep_feature_import',
    source: 'src/onboarding/a.ts',
    target: 'src/settings/b.ts',
  }])
})
