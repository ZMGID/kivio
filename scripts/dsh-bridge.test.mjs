import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import test from 'node:test'

const source = readFileSync(new URL('../src-tauri/resources/dsh/kivio-dsh-bridge.mjs', import.meta.url), 'utf8')

function fixture(preset = 'standard', previous = { provider: 'deepseek-official', model: 'flash', reasoningEffort: 'low' }) {
  const scope = {
    HarnessSdkJsonRpcServer: class {},
    createUserMessage: message => ({ ...message, id: 'message-1' }),
    process: { env: { DSH_AGENT_PRESET: preset } },
    console,
  }
  vm.runInNewContext(source.slice(0, source.indexOf('export const name'))
    .replace(/^import .*$/gm, '').replace('export class ', 'class ')
    + '\nglobalThis.Bridge = KivioHarnessSdkJsonRpcServer', scope)
  const server = Object.create(scope.Bridge.prototype)
  const mounted = []
  const injected = []
  let selection = previous
  const agent = {
    id: 'session-1',
    session: { header: { cwd: 'D:/project', agentPreset: preset } },
    model: { current: () => selection, select: value => { selection = value } },
    steer: message => injected.push(message),
    followup: () => { throw new Error('steering must not enqueue a follow-up') },
  }
  Object.assign(server, {
    provider: 'deepseek-official', model: 'flash', sessions: new Map(),
    lifetime: new AbortController(),
    assertLiveAgent: () => {},
    attachToWorkspace: async () => {},
    ctx: {
      agents: { get: () => agent, resume: async options => {
        await options.setup({ agent }, agent)
        return { agent }
      } },
      get: name => name === 'agentPresets' ? { mount: async (_ctx, id) => mounted.push(id), presetForSession: session => session.selectedPreset ?? session.header.agentPreset } : undefined,
    },
  })
  return { server, agent, mounted, injected }
}

test('custom presets reach the registry; old code sessions mount ptc', async () => {
  for (const [saved, expected] of [['my-reviewer', 'my-reviewer'], ['code', 'ptc']]) {
    const { server, mounted } = fixture(saved)
    assert.equal(server.agentPreset(), expected)
    await server.resumeSession('session-1')
    assert.deepEqual(mounted, [expected])
  }
  assert.throws(() => fixture('../outside').server.agentPreset(), /invalid/)
})

test('resume updates the durable model and explicit effort', async () => {
  const { server, agent } = fixture()
  server.model = 'pro'
  server.reasoningEffort = 'max'
  await server.resumeSession('session-1')
  assert.equal(agent.model.current().model, 'pro')
  assert.equal(agent.model.current().reasoningEffort, 'max')
})

test('auto effort survives same-route resume but does not leak into another model', async () => {
  for (const [model, effort] of [['flash', 'low'], ['pro', undefined]]) {
    const { server, agent } = fixture()
    server.model = model
    await server.resumeSession('session-1')
    assert.equal(agent.model.current().reasoningEffort, effort)
  }
})

test('steer uses the native inbox without replacing followup', async () => {
  const { server, agent, injected } = fixture()
  await server.resumeSession('session-1')
  const followup = agent.followup
  const result = await server.steer({ sessionId: 'session-1', contentBlocks: [{ type: 'text', text: 'change direction' }] })
  assert.equal(result.messageId, 'message-1')
  assert.equal(injected[0].content[0].text, 'change direction')
  assert.equal(agent.followup, followup)
})

test('resume follows the native preset projection after a selection changes', async () => {
  const { server, agent, mounted } = fixture('standard')
  agent.session.selectedPreset = 'minimal'
  await server.resumeSession('session-1')
  assert.deepEqual(mounted, ['minimal'])
})
