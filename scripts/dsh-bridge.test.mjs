import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import test from 'node:test'

const source = readFileSync(new URL('../src-tauri/resources/dsh/kivio-dsh-bridge.mjs', import.meta.url), 'utf8')

function fixture(preset = 'standard', previous = { provider: 'deepseek-official', model: 'flash', reasoningEffort: 'low' }) {
  const scope = {
    HarnessSdkJsonRpcServer: class {
      async prompt(params) {
        const record = await this.getOrCreateSession(params.sessionId)
        record.handle.agent.followup({ content: params.contentBlocks })
        return { messageId: 'prompt-1' }
      }
    },
    Buffer,
    createUserMessage: message => ({ ...message, id: 'message-1' }),
    process: { env: { DSH_AGENT_PRESET: preset, DSH_PERMISSION_MODE: 'read-only' } },
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
    session: { header: { cwd: 'D:/project', agentPreset: preset }, permissionPreset: 'danger-full-access' },
    model: { current: () => selection, select: value => { selection = value } },
    steer: message => injected.push(message),
    followup: () => { throw new Error('steering must not enqueue a follow-up') },
  }
  const services = {
    agentPresets: { mount: async (_ctx, id) => mounted.push(id), presetForSession: session => session.selectedPreset ?? session.header.agentPreset },
    permissionPresets: { set: (session, mode) => { session.permissionPreset = mode } },
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
      get: name => services[name],
    },
  })
  return { server, agent, mounted, injected, services }
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

test('resume replaces stored full access with the current read-only selection', async () => {
  const { server, agent } = fixture()
  await server.resumeSession('session-1')
  assert.equal(agent.session.permissionPreset, 'read-only')
})

test('stop job passes the owning session id to the native registry', async () => {
  const { server, agent } = fixture()
  await server.resumeSession('session-1')
  let stopped = false
  server.ctx.jobs = { list: caller => {
    assert.equal(caller, agent.id)
    return [{ id: 'bash-1' }]
  }, kill: (id, caller) => {
    assert.equal(id, 'bash-1')
    assert.equal(caller, agent.id)
    stopped = true
    return 'requested'
  } }
  assert.equal((await server.stopJob({ sessionId: agent.id, jobId: 'bash-1' })).outcome, 'requested')
  assert.equal(stopped, true)
})

test('a refused child interruption cannot fall through to direct cancellation', async () => {
  const { server, agent } = fixture()
  await server.resumeSession('session-1')
  let cancelled = false
  server.ctx.subagents = { interrupt: () => { throw new Error('belongs to another parent session') } }
  server.ctx.agents.get = () => ({ cancel: () => { cancelled = true } })
  await assert.rejects(server.stopJob({ sessionId: agent.id, jobId: 'foreign-child' }), /another parent/)
  assert.equal(cancelled, false)
})

test('an image upload cannot be overtaken by a later prompt or steer', async () => {
  for (const firstMethod of ['prompt', 'steer']) {
    const { server, agent, services } = fixture()
    await server.resumeSession('session-1')
    const started = Promise.withResolvers()
    const uploaded = Promise.withResolvers()
    const delivered = []
    agent.followup = message => delivered.push(message.content[0].text)
    agent.steer = agent.followup
    services.attachments = { saveImage: async () => {
      started.resolve()
      await uploaded.promise
      return { attachmentId: 'image-1', mediaType: 'image/png' }
    } }
    const first = server[firstMethod]({ sessionId: agent.id, contentBlocks: [
      { type: 'text', text: 'first' }, { type: 'image', mediaType: 'image/png', data: 'aGVsbG8=' },
    ] })
    await started.promise
    const second = server.prompt({ sessionId: agent.id, contentBlocks: [{ type: 'text', text: 'second' }] })
    const third = server.steer({ sessionId: agent.id, contentBlocks: [{ type: 'text', text: 'third' }] })
    await new Promise(resolve => setImmediate(resolve))
    uploaded.resolve()
    await Promise.all([first, second, third])
    assert.deepEqual(delivered, ['first', 'second', 'third'])
  }
})

test('an image upload failure does not stall later input', async () => {
  const { server, agent, services } = fixture()
  await server.resumeSession('session-1')
  services.attachments = { saveImage: async () => { throw new Error('disk full') } }
  const delivered = []
  agent.followup = message => delivered.push(message.content[0].text)
  const failed = assert.rejects(server.prompt({ sessionId: agent.id, contentBlocks: [
    { type: 'image', mediaType: 'image/png', data: 'aGVsbG8=' },
  ] }), /disk full/)
  await server.prompt({ sessionId: agent.id, contentBlocks: [{ type: 'text', text: 'next' }] })
  await failed
  assert.deepEqual(delivered, ['next'])
})

test('shutdown during image upload prevents steering a disposed session', async () => {
  const { server, agent, services, injected } = fixture()
  await server.resumeSession('session-1')
  const started = Promise.withResolvers()
  const uploaded = Promise.withResolvers()
  services.attachments = { saveImage: async () => {
    started.resolve()
    await uploaded.promise
    return { attachmentId: 'image-1', mediaType: 'image/png' }
  } }
  const rejected = assert.rejects(server.steer({ sessionId: agent.id, contentBlocks: [
    { type: 'image', mediaType: 'image/png', data: 'aGVsbG8=' },
  ] }), /shutdown/)
  await started.promise
  server.lifetime.abort(new Error('shutdown'))
  uploaded.resolve()
  await rejected
  assert.equal(injected.length, 0)
})

test('native job cancellation failures are reported without trying another target', async () => {
  const { server, agent } = fixture()
  await server.resumeSession('session-1')
  server.ctx.jobs = { list: () => [{ id: 'bash-1' }], kill: () => { throw new Error('kill failed') } }
  let interrupted = false
  server.ctx.subagents = { interrupt: () => { interrupted = true } }
  await assert.rejects(server.stopJob({ sessionId: agent.id, jobId: 'bash-1' }), /kill failed/)
  assert.equal(interrupted, false)
})
