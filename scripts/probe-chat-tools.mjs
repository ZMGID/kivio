#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'

const identifier = 'com.zmair.kivio.test'
const owner = { kind: 'kivio-stack-fixture', identifier, version: 1 }
const options = {}
for (let index = 2; index < process.argv.length; index++) {
  const argument = process.argv[index]
  if (['--exe', '--app-data', '--timeout-seconds'].includes(argument)) {
    const value = process.argv[++index]
    assert(value && !value.startsWith('--'), `${argument} requires a value`)
    options[argument.slice(2)] = value
  } else {
    assert(['--help', '--self-test'].includes(argument), `Unknown option ${argument}`)
    options[argument.slice(2)] = true
  }
}

function readJson(file) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')) }
  catch (error) {
    if (error.code === 'ENOENT') return null
    throw error
  }
}

function claimFixture(directory) {
  assert.equal(path.basename(directory), identifier, 'Only the isolated Test identity is allowed')
  const marker = path.join(directory, '.kivio-stack-fixture.json')
  if (fs.existsSync(directory)) {
    const saved = readJson(marker)
    assert(saved && Object.entries(owner).every(([key, value]) => saved[key] === value), 'Existing app-data must have this fixture ownership marker')
    if (saved.pid) {
      let alive = true
      try { process.kill(saved.pid, 0) } catch (error) {
        if (error.code === 'ESRCH') alive = false
        else throw error
      }
      assert(!alive, 'A previous fixture process is still running')
    }
    const document = readJson(path.join(directory, 'settings.json'))
    if (document) {
      assert(document.settings && typeof document.settings === 'object', 'Unrecognized settings document')
      for (const provider of document.settings.providers ?? []) {
        assert((provider.apiKeys ?? []).every((key) => !key || key === 'fixture-only'), 'Refusing to replace settings with real provider credentials')
        assert(!provider.apiKey || provider.apiKey === 'fixture-only', 'Refusing to replace settings with a legacy provider credential')
      }
      const search = document.settings.lens?.webSearch ?? {}
      assert(!Object.entries(search).some(([key, value]) => /apiKey|Auth$/i.test(key) && value), 'Refusing to replace search credentials')
      assert(!(document.settings.chatTools?.servers ?? []).some((server) => server.auth || Object.keys(server.headers ?? {}).length), 'Refusing to replace authenticated MCP configuration')
    }
  } else {
    fs.mkdirSync(directory, { recursive: true })
    fs.writeFileSync(marker, JSON.stringify(owner), { flag: 'wx' })
  }
  return marker
}

function publishRequest(directory, request) {
  const temporary = path.join(directory, `${request.id}.tmp`)
  fs.writeFileSync(temporary, JSON.stringify(request), { flag: 'wx' })
  try {
    fs.linkSync(temporary, path.join(directory, 'request.json'))
  } finally {
    fs.unlinkSync(temporary)
  }
}

function jsonResponse(response, status, value, headers = {}) {
  response.writeHead(status, { 'content-type': 'application/json', ...headers })
  response.end(value === undefined ? undefined : JSON.stringify(value))
}

function messageText(message) {
  return typeof message.content === 'string' ? message.content
    : (message.content ?? []).map((part) => part.text ?? '').join('\n')
}

function modelReply(body, text, calls = []) {
  const message = calls.length
    ? { role: 'assistant', content: null, tool_calls: calls.map(({ name, arguments: args }, index) => ({ id: `fixture-${index}-${randomUUID()}`, type: 'function', function: { name, arguments: JSON.stringify(args) } })) }
    : { role: 'assistant', content: text }
  return { id: 'fixture-completion', object: 'chat.completion', created: 0, model: body.model,
    choices: [{ index: 0, message, finish_reason: calls.length ? 'tool_calls' : 'stop' }],
    usage: { prompt_tokens: 100, completion_tokens: 20, total_tokens: 120 } }
}

function sendModel(response, body, reply) {
  if (!body.stream) return jsonResponse(response, 200, reply)
  response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' })
  const choice = reply.choices[0]
  const delta = choice.message.tool_calls
    ? { role: 'assistant', tool_calls: choice.message.tool_calls.map((call, index) => ({ index, ...call })) }
    : { role: 'assistant', content: choice.message.content }
  for (const event of [
    { choices: [{ index: 0, delta }] },
    { choices: [{ index: 0, delta: {}, finish_reason: choice.finish_reason }], usage: reply.usage },
    '[DONE]',
  ]) response.write(`data: ${typeof event === 'string' ? event : JSON.stringify(event)}\n\n`)
  response.end()
}

async function startMocks(record = () => {}) {
  const stats = { model: [], mcp: { initialize: 0, list: 0, calls: [] }, exa: { initialize: 0, list: 0, calls: [] }, maxParallel: 0, errors: [] }
  const pending = new Map()
  let parallel = 0
  const tool = { name: 'test_echo', description: 'Kivio stack fixture echo',
    inputSchema: { type: 'object', properties: { value: { type: 'string' } }, required: ['value'] },
    annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false } }
  const server = http.createServer(async (request, response) => {
    try {
      const route = new URL(request.url, 'http://localhost').pathname
      if (request.method === 'GET' && route === '/v1/models') return jsonResponse(response, 200, { object: 'list', data: [{ id: 'test-model', object: 'model', owned_by: 'fixture' }] })
      if (request.method === 'DELETE') return jsonResponse(response, 200)
      if (request.method !== 'POST') return jsonResponse(response, 405, { error: 'POST only' })
      let raw = ''
      for await (const chunk of request) {
        raw += chunk
        assert(raw.length < 10 * 1024 * 1024, 'Fixture request is too large')
      }
      const body = JSON.parse(raw)
      if (route === '/v1/chat/completions') {
        const messages = body.messages ?? []
        const userIndex = messages.findLastIndex((message) => message.role === 'user' && messageText(message).includes('[kivio-stack:'))
        const marker = userIndex < 0 ? null : messageText(messages[userIndex]).match(/\[kivio-stack:([a-z0-9-]+):([a-z-]+)\]/)
        const functions = (body.tools ?? []).map((item) => item.function)
        if (!marker || (!functions.length && !messages.slice(userIndex + 1).some((message) => message.role === 'tool'))) {
          return sendModel(response, body, modelReply(body, 'Fixture title'))
        }
        const scenario = marker[2]
        const token = `${marker[1]}:${scenario}`
        const results = messages.slice(userIndex + 1).filter((message) => message.role === 'tool')
        const native = functions.find((item) => item.description?.includes('Search the web for current facts'))
        const direct = functions.find((item) => item.description === tool.description)
        const observed = { scenario, results: results.length, tools: functions.map((item) => item.name) }
        stats.model.push(observed)
        record({ type: 'model', ...observed })
        let calls = []
        if (!results.length) {
          if (['native', 'parallel'].includes(scenario)) {
            assert(native, 'Native web_search was not advertised to the model')
            calls.push({ name: native.name, arguments: { query: token } })
          }
          if (scenario !== 'native') {
            assert(direct, 'Direct MCP test_echo was not advertised to the model')
            calls.push({ name: direct.name, arguments: { value: `${token}${scenario === 'failure' ? ':fail' : ''}` } })
          }
        } else if (scenario === 'failure' && results.length === 1) {
          assert(direct, 'Retry lost the advertised MCP tool')
          calls = [{ name: direct.name, arguments: { value: `${token}:retry` } }]
        }
        return sendModel(response, body, modelReply(body, `fixture-completed-${scenario}`, calls))
      }
      assert(['/mcp', '/exa'].includes(route), `Unexpected mock route ${route}`)
      const kind = route.slice(1)
      const counters = stats[kind]
      record({ type: 'mcp', route, method: body.method, name: body.params?.name, arguments: body.params?.arguments })
      if (!Object.hasOwn(body, 'id')) return jsonResponse(response, 202)
      const answer = (result) => jsonResponse(response, 200, { jsonrpc: '2.0', id: body.id, result }, { 'mcp-session-id': `fixture-${kind}` })
      if (body.method === 'initialize') {
        counters.initialize++
        return answer({ protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: `fixture-${kind}`, version: '1.0.0' } })
      }
      if (body.method === 'tools/list') {
        counters.list++
        return answer({ tools: kind === 'mcp' ? [tool] : [{ ...tool, name: 'web_search_exa', inputSchema: { type: 'object', properties: { query: { type: 'string' }, numResults: { type: 'integer' } }, required: ['query'] } }] })
      }
      if (body.method === 'ping') return answer({})
      assert.equal(body.method, 'tools/call', 'Unexpected MCP request method')
      assert.equal(body.params.name, kind === 'mcp' ? 'test_echo' : 'web_search_exa')
      const value = kind === 'mcp' ? body.params.arguments.value : body.params.arguments.query
      assert.equal(typeof value, 'string')
      const call = { name: body.params.name, value, failed: value.endsWith(':fail'), pending: value.endsWith(':cancelled') }
      counters.calls.push(call)
      const result = { content: [{ type: 'text', text: kind === 'exa'
        ? JSON.stringify({ results: [{ title: 'Fixture result', url: 'https://fixture.invalid/result', text: value }] })
        : call.failed ? 'fixture intentional tool failure' : `fixture echo ${value}` }], isError: call.failed }
      if (call.pending) {
        pending.set(`${kind}-${body.id}`, () => { if (!response.destroyed) answer(result) })
        return
      }
      if (value.endsWith(':parallel')) {
        parallel++
        stats.maxParallel = Math.max(stats.maxParallel, parallel)
        setTimeout(() => { parallel--; if (!response.destroyed) answer(result) }, 500)
        return
      }
      answer(result)
    } catch (error) {
      stats.errors.push(error.message)
      record({ type: 'fixture-error', error: error.message })
      if (!response.headersSent) jsonResponse(response, 500, { error: { message: error.message, type: 'fixture_error' } })
      else response.destroy()
    }
  })
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
  return { base: `http://127.0.0.1:${server.address().port}`, stats, pending,
    releasePending() { for (const release of pending.values()) release(); pending.clear() },
    async close() { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)) } }
}

function fixtureSettings(base, workdir) {
  return { settings: {
    hotkey: '', chatHotkey: '', closeChatHotkey: '', autoCheckUpdate: false,
    launchAtStartup: false, launchMinimizedToTray: false, onboardingStatus: 'completed',
    retryEnabled: false, retryAttempts: 1, chatToolsGreenlitV1: true, chatBehaviorMigratedFromLens: true,
    providers: [{ id: 'stack-fixture', name: 'Stack fixture', enabled: true, apiFormat: 'openai_chat',
      apiKeys: ['fixture-only'], baseUrl: `${base}/v1`, availableModels: ['test-model'], enabledModels: ['test-model'] }],
    defaultModels: { chat: { providerId: 'stack-fixture', model: 'test-model' } },
    chatProviderId: 'stack-fixture', chatModel: 'test-model', translatorProviderId: 'stack-fixture', translatorModel: 'test-model',
    chat: { thinkingEnabled: false, streamEnabled: true, defaultAgentRuntime: { kind: 'builtin' } },
    chatMemory: { enabled: false },
    lens: { webSearch: { enabled: true, provider: 'exa_mcp', exaMcpUrl: `${base}/exa` } },
    chatTools: { enabled: true, approvalPolicy: 'auto', skillAutoMatch: false, toolTimeoutMs: 15000,
      servers: [{ id: 'stack-fixture', name: 'Stack fixture', enabled: true, transport: 'streamable_http', url: `${base}/mcp`, enabledTools: ['test_echo'] }],
      nativeTools: { webSearch: true, webFetch: false, skillRuntime: false, readFile: false, writeFile: false, editFile: false,
        runCommand: false, knowledgeSearch: false, automation: false, scheduledTasks: false, workingDirectory: workdir } },
  } }
}

async function selfTest() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kivio-tools-self-test-'))
  const mocks = await startMocks()
  try {
    const fixture = path.join(directory, identifier)
    claimFixture(fixture)
    const settingsFile = path.join(fixture, 'settings.json')
    fs.writeFileSync(settingsFile, JSON.stringify({ settings: { providers: [{ apiKey: 'fixture-only' }] } }))
    claimFixture(fixture)
    fs.writeFileSync(settingsFile, JSON.stringify({ settings: { providers: [{ apiKey: 'not-a-fixture' }] } }))
    assert.throws(() => claimFixture(fixture), /legacy provider credential/)
    assert.throws(() => claimFixture(path.join(directory, 'com.zmair.kivio.dev')))
    const requests = path.join(fixture, 'chat_probe')
    fs.mkdirSync(requests)
    publishRequest(requests, { id: 'first' })
    assert.throws(() => publishRequest(requests, { id: 'second' }), { code: 'EEXIST' })
    assert.equal(readJson(path.join(requests, 'request.json')).id, 'first')
    const rpc = async (method, params = {}) => {
      const response = await fetch(`${mocks.base}/mcp`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: randomUUID(), method, params }) })
      assert.equal(response.status, 200)
      return (await response.json()).result
    }
    assert.equal((await rpc('initialize')).protocolVersion, '2025-06-18')
    assert.equal((await rpc('tools/list')).tools[0].name, 'test_echo')
    assert.equal((await rpc('tools/call', { name: 'test_echo', arguments: { value: 'self-test' } })).isError, false)
    assert.equal((await rpc('tools/call', { name: 'test_echo', arguments: { value: 'self-test:fail' } })).isError, true)
    const response = await fetch(`${mocks.base}/v1/chat/completions`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ model: 'test-model', stream: true, messages: [{ role: 'user', content: '[kivio-stack:self-test:direct]' }], tools: [{ type: 'function', function: { name: 'discovered_echo', description: 'Kivio stack fixture echo' } }] }) })
    const events = await response.text()
    assert(events.includes('discovered_echo') && events.includes('[DONE]'))
    assert.deepEqual(mocks.stats.errors, [])
    console.log('PASS isolated ownership, no-overwrite publication, MCP protocol, and model SSE')
  } finally {
    await mocks.close()
    fs.rmSync(directory, { recursive: true, force: true })
  }
}

if (options.help) {
  console.log('node scripts/probe-chat-tools.mjs --exe PATH --app-data "%APPDATA%\\com.zmair.kivio.test" [--timeout-seconds 120]')
  console.log('Requires a debug Test-identity build. Starts only local model/MCP fixtures, retains evidence, and never writes Dev or production settings.')
} else if (options['self-test']) {
  await selfTest()
} else {
  assert(options.exe && options['app-data'], '--exe and --app-data are required')
  assert.equal(process.platform, 'win32', 'This native-stack regression targets Windows')
  const appData = path.resolve(options['app-data'])
  const expected = path.resolve(process.env.APPDATA, identifier)
  assert.equal(appData.toLowerCase(), expected.toLowerCase(), 'App-data must be APPDATA/com.zmair.kivio.test')
  const exe = fs.realpathSync(options.exe)
  const product = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', '[Diagnostics.FileVersionInfo]::GetVersionInfo($env:KIVIO_FIXTURE_EXE).ProductName'], {
    windowsHide: true, encoding: 'utf8', timeout: 5000, env: { ...process.env, KIVIO_FIXTURE_EXE: exe },
  })
  assert.equal(product.status, 0, 'Could not read executable identity')
  assert.equal(product.stdout.trim(), 'Kivio Test', 'Refusing to launch a non-Test executable')
  const compiledIdentity = spawnSync(exe, ['--print-app-identifier'], { windowsHide: true, encoding: 'utf8', timeout: 5000 })
  assert.equal(compiledIdentity.status, 0, 'Build the current Test executable with identity-query support')
  assert.equal(compiledIdentity.stdout.trim(), identifier, 'Executable identifier does not match isolated app-data')
  const marker = claimFixture(appData)
  assert.equal(fs.realpathSync(appData).toLowerCase(), appData.toLowerCase(), 'Fixture directory must not redirect through a junction')
  const timeoutSeconds = Number(options['timeout-seconds'] ?? 120)
  assert(Number.isSafeInteger(timeoutSeconds) && timeoutSeconds >= 10 && timeoutSeconds <= 3600, 'Timeout must be between 10 and 3600 seconds')
  const runId = `${Date.now()}-${randomUUID().slice(0, 8)}`
  const artifacts = path.join(appData, 'stack-fixture', runId)
  const probeDir = path.join(appData, 'chat_probe')
  const workdir = path.join(artifacts, 'work')
  fs.mkdirSync(workdir, { recursive: true })
  fs.mkdirSync(probeDir, { recursive: true })
  assert(!fs.existsSync(path.join(probeDir, 'request.json')), 'A probe request is pending')
  const record = (event) => fs.appendFileSync(path.join(artifacts, 'protocol.jsonl'), `${JSON.stringify(event)}\n`)
  const mocks = await startMocks(record)
  let child
  let spawnError
  const summary = { runId, identifier, exe, scenarios: [], passed: false }
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const alive = () => {
    if (spawnError) throw spawnError
    assert(child && child.exitCode === null && child.signalCode === null && !child.killed, `Fixture app exited with ${child?.exitCode ?? child?.signalCode}; inspect app.stderr.log`)
  }
  async function until(read, timeout, description) {
    const deadline = Date.now() + timeout
    do {
      alive()
      assert.deepEqual(mocks.stats.errors, [], 'Mock protocol failed; inspect protocol.jsonl')
      const value = read()
      if (value) return value
      await sleep(100)
    } while (Date.now() < deadline)
    throw new Error(`Timeout waiting for ${description}`)
  }
  let conversationId
  let sequence = 0
  let cancelledAssistant
  async function turn(scenario, cancelAfterMs) {
    const id = `tools-${runId}-${++sequence}-${scenario}`
    const prompt = `[kivio-stack:${runId}:${scenario}] Execute the fixture tool step, then report completion.`
    const request = { id, prompt, sendThroughIpc: true, provider: 'stack-fixture', model: 'test-model', cwd: workdir,
      webSearchMode: 'third_party', timeoutSeconds, ...(conversationId ? { conversationId } : {}),
      ...(cancelAfterMs === undefined ? {} : { cancelAfterMs }) }
    publishRequest(probeDir, request)
    await until(() => !fs.existsSync(path.join(probeDir, 'request.json')), 15000, 'request consumption')
    const result = await until(() => readJson(path.join(probeDir, `result-${id}.json`)), timeoutSeconds * 1000 + 15000, `${scenario} IPC result`)
    assert.equal(result.id, id)
    assert(!result.error, `Probe ${scenario} failed; inspect result-${id}.json`)
    if (conversationId) assert.equal(result.conversationId, conversationId)
    conversationId = result.conversationId
    assert(conversationId, 'Missing persisted conversation ID')
    assert.equal(result.streamOutcome, scenario === 'cancelled' ? 'cancelled' : 'completed')
    if (scenario !== 'cancelled') assert.equal(result.answer, `fixture-completed-${scenario}`)
    const saved = readJson(path.join(appData, 'conversations', `${conversationId}.json`))
    assert.equal(saved.agent_runtime.kind, 'builtin')
    assert.equal(saved.messages.filter((message) => message.role === 'user').length, sequence, 'One persisted user message per IPC send')
    const userIndex = saved.messages.findLastIndex((message) => message.role === 'user')
    assert.equal(saved.messages[userIndex].content, prompt)
    const assistants = saved.messages.slice(userIndex + 1).filter((message) => message.role === 'assistant')
    assert.equal(assistants.length, 1, 'The current user must have exactly one persisted assistant')
    const assistant = assistants[0]
    assert.equal(assistant.stream_outcome, result.streamOutcome)
    const records = assistant.tool_calls ?? []
    assert(records.length, 'A pure-text response cannot pass the tool fixture')
    const native = records.filter((call) => call.source === 'native' && call.name === 'web_search')
    const direct = records.filter((call) => call.source === 'mcp' && call.name === 'test_echo')
    if (['native', 'parallel'].includes(scenario)) assert(native.some((call) => call.status === 'success'), 'Missing successful persisted native web_search')
    if (scenario !== 'native' && scenario !== 'cancelled') assert(direct.some((call) => call.status === 'success'), 'Missing successful persisted direct MCP tool')
    if (scenario === 'failure') {
      assert(direct.some((call) => call.status === 'error'), 'The intentional MCP failure was not persisted')
      assert(mocks.stats.mcp.calls.some((call) => call.value.endsWith(':failure:fail') && call.failed))
      assert(mocks.stats.mcp.calls.some((call) => call.value.endsWith(':failure:retry') && !call.failed))
    }
    if (scenario === 'parallel') assert(mocks.stats.maxParallel >= 2, 'The two safe tools did not overlap on the MCP server')
    if (scenario === 'cancelled') {
      assert(mocks.stats.mcp.calls.some((call) => call.pending), 'Cancellation did not reach a pending MCP tool')
      assert.equal(direct.length, 1, 'Cancellation must include the pending MCP tool record')
      assert.equal(direct[0].status, 'error', 'An in-flight MCP cancellation must preserve its unknown remote outcome')
      assert.match(direct[0].error, /^MCP request cancelled; request outcome is unknown and was not retried/)
      assert.equal(mocks.stats.mcp.calls.filter((call) => call.pending).length, 1, 'The cancelled remote request must not be replayed')
      cancelledAssistant = assistant
      mocks.releasePending()
    }
    if (scenario === 'recovery') {
      assert.deepEqual(saved.messages.find((message) => message.id === cancelledAssistant.id), cancelledAssistant, 'A late tool response changed the cancelled turn')
    }
    summary.scenarios.push({ scenario, conversationId, outcome: result.streamOutcome, records: records.map(({ name, source, status }) => ({ name, source, status })) })
    console.log(`PASS ${scenario}`)
  }
  try {
    fs.writeFileSync(path.join(appData, 'settings.json'), JSON.stringify(fixtureSettings(mocks.base, workdir), null, 2))
    const stdout = fs.openSync(path.join(artifacts, 'app.stdout.log'), 'wx')
    const stderr = fs.openSync(path.join(artifacts, 'app.stderr.log'), 'wx')
    try { child = spawn(exe, [], { cwd: path.dirname(exe), windowsHide: true, stdio: ['ignore', stdout, stderr] }) }
    finally { fs.closeSync(stdout); fs.closeSync(stderr) }
    child.on('error', (error) => { spawnError = error })
    fs.writeFileSync(marker, JSON.stringify({ ...owner, pid: child.pid, runId }))
    await until(() => fs.readFileSync(path.join(artifacts, 'app.stderr.log'), 'utf8').includes(`[chat-probe] watching ${path.join(probeDir, 'request.json')}`), 90000, 'Test app probe watcher')
    await sleep(3500)
    alive()
    await turn('native')
    await turn('direct')
    await turn('parallel')
    await turn('cancelled', 3000)
    await turn('recovery')
    await turn('failure')
    for (const kind of ['mcp', 'exa']) {
      assert(mocks.stats[kind].initialize > 0, `${kind} initialize was not exercised`)
      assert(mocks.stats[kind].calls.length > 0, `${kind} tools/call was not exercised`)
    }
    assert(mocks.stats.mcp.list > 0, 'Direct MCP discovery did not call tools/list')
    const stack = [...fs.readFileSync(path.join(artifacts, 'app.stderr.log'), 'utf8').matchAll(/point=MCP tool connecting .*total_bytes=(\d+) used_bytes=(\d+) remaining_bytes=(\d+)/g)]
      .map(([, total, used, remaining]) => ({ total: Number(total), used: Number(used), remaining: Number(remaining) }))
    assert(stack.length >= 2, 'Missing native MCP connection stack samples')
    assert(stack.every((sample) => sample.total === 2 * 1024 * 1024), 'The regression must use the default 2 MiB worker stack')
    summary.stack = { samples: stack.length, maxUsedAtMcpConnect: Math.max(...stack.map((sample) => sample.used)), minRemainingAtMcpConnect: Math.min(...stack.map((sample) => sample.remaining)) }
    assert(summary.stack.minRemainingAtMcpConnect >= 1024 * 1024, 'Native MCP connection must start with at least 1 MiB of stack headroom')
    summary.passed = true
    console.log(`PASS native Exa MCP, direct MCP, parallel tools, pending cancellation/recovery, and tool failure/retry. Evidence ${artifacts}`)
  } catch (error) {
    summary.error = error.message
    process.exitCode = 1
    console.error(`FAIL ${error.message}. Evidence ${artifacts}`)
  } finally {
    summary.stats = mocks.stats
    fs.writeFileSync(path.join(artifacts, 'summary.json'), JSON.stringify(summary, null, 2))
    mocks.releasePending()
    if (child && child.exitCode === null && child.signalCode === null) {
      child.kill()
      await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5000)])
    }
    await mocks.close()
    fs.writeFileSync(marker, JSON.stringify({ ...owner, pid: child?.exitCode === null && child?.signalCode === null ? child.pid : null, runId }))
  }
}
