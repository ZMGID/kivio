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
      kind: 'module_cycle',
      modules: ['chat', 'settings'],
      cycle: ['chat', 'settings', 'chat'],
      witness: [
        { source: 'src/chat/public/a.ts', line: 1, specifier: '../../settings/public/b', target: 'src/settings/public/b.ts', type: 're-export' },
        { source: 'src/settings/public/b.ts', line: 1, specifier: '../../chat/public/a', target: 'src/chat/public/a.ts', type: 're-export' },
      ],
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
      kind: 'adapter_to_feature',
      source: 'src/api/bridge.ts',
      target: 'src/settings/public/b.ts',
    },
    {
      kind: 'cross_feature_cycle',
      source: 'src/api/bridge.ts',
      target: 'src/api/bridge.ts',
      cycle: ['src/api/bridge.ts', 'src/chat/public/a.ts', 'src/settings/public/b.ts'],
    },
    {
      kind: 'module_cycle',
      modules: ['api', 'chat', 'settings'],
      cycle: ['api', 'settings', 'chat', 'api'],
      witness: [
        { source: 'src/api/bridge.ts', line: 1, specifier: '../settings/public/b', target: 'src/settings/public/b.ts', type: 're-export' },
        { source: 'src/settings/public/b.ts', line: 1, specifier: '../../chat/public/a', target: 'src/chat/public/a.ts', type: 're-export' },
        { source: 'src/chat/public/a.ts', line: 1, specifier: '../../api/bridge', target: 'src/api/bridge.ts', type: 're-export' },
      ],
    },
    {
      kind: 'public_reverse_dependency',
      source: 'src/settings/public/b.ts',
      target: 'src/chat/public/a.ts',
    },
  ])
})

test('finds a dispersed Module cycle with a precise type-only, dynamic and re-export witness', () => {
  const root = fixture({
    'src/chat/consumer.ts': "import type { Setting } from '@/settings/public/types'\nexport type ChatSetting = Setting",
    'src/chat/other.ts': 'export const chat = 1',
    'src/settings/public/types.ts': 'export type Setting = string',
    'src/settings/loader.ts': "void import('@/api/gateway')",
    'src/api/gateway.ts': "export { chat } from '../chat/other'",
  })

  const cycles = newViolations(root, { existingViolations: [] })
    .filter((violation) => violation.kind === 'module_cycle')
  assert.deepEqual(cycles, [{
    kind: 'module_cycle',
    modules: ['api', 'chat', 'settings'],
    cycle: ['api', 'chat', 'settings', 'api'],
    witness: [
      { source: 'src/api/gateway.ts', line: 1, specifier: '../chat/other', target: 'src/chat/other.ts', type: 're-export' },
      { source: 'src/chat/consumer.ts', line: 1, specifier: '@/settings/public/types', target: 'src/settings/public/types.ts', type: 'type-only' },
      { source: 'src/settings/loader.ts', line: 1, specifier: '@/api/gateway', target: 'src/api/gateway.ts', type: 'dynamic' },
    ],
  }])
})

test('fails closed when a new src path has no declared Module', () => {
  const root = fixture({ 'src/unclaimed/entry.ts': 'export const entry = 1' })
  assert.deepEqual(newViolations(root, {
    modules: [{ name: 'chat', role: 'feature', paths: ['src/chat/**'] }],
    existingViolations: [],
  }), [{ kind: 'unknown_module_path', source: 'src/unclaimed/entry.ts', target: 'src/unclaimed/entry.ts' }])
})

test('CLI diagnostic prints every exact witness edge instead of undefined endpoints', async () => {
  const { formatViolation } = await import('./check-architecture.mjs')
  const formatted = formatViolation({
    kind: 'module_cycle', modules: ['api', 'chat'], cycle: ['api', 'chat', 'api'], witness: [
      { source: 'src/api/bridge.ts', line: 4, specifier: '../chat/public/x', target: 'src/chat/public/x.ts', type: 're-export' },
      { source: 'src/chat/a.ts', line: 9, specifier: '../api/bridge', target: 'src/api/bridge.ts', type: 'type-only' },
    ],
  })
  assert.equal(formatted, [
    '[module_cycle] api -> chat -> api (SCC: api, chat)',
    '    [re-export] src/api/bridge.ts:4 -> src/chat/public/x.ts (../chat/public/x)',
    '    [type-only] src/chat/a.ts:9 -> src/api/bridge.ts (../api/bridge)',
  ].join('\n'))
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
