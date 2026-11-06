#!/usr/bin/env node
// Run against an already running debug app. Conversations and evidence are retained.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { randomUUID } from 'node:crypto'
import { fileURLToPath } from 'node:url'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const scenarios = ['batch-scouts', 'idle-auto-result', 'nested-irc', 'task-error', 'cancel-resume', 'eval-shared', 'workpool-reuse']
const options = {}
const filters = []
for (let index = 2; index < process.argv.length; index++) {
  const arg = process.argv[index]
  if (['--probe-dir', '--provider', '--model'].includes(arg)) {
    const value = process.argv[++index]
    assert(value && !value.startsWith('--'), `${arg} requires a value`)
    options[arg.slice(2)] = value
  } else if (['--list', '--self-test', '--help'].includes(arg)) options[arg.slice(2)] = true
  else {
    assert(!arg.startsWith('--'), `Unknown option ${arg}`)
    filters.push(arg)
  }
}

function journal(text) {
  const lines = text.split('\n')
  lines.pop() // A writer may still be appending the final line.
  return lines.filter((line) => line.trim()).map((line) => JSON.parse(line))
}

function calls(messages) {
  return messages.flatMap((message) => message.tool_calls ?? []).map((call) => ({
    ...call,
    name: (call.name ?? call.function?.name ?? '').replace(/^(native|eval)__/, ''),
    args: typeof (call.arguments ?? call.function?.arguments) === 'string'
      ? JSON.parse(call.arguments ?? call.function.arguments)
      : (call.arguments ?? call.function?.arguments ?? {}),
  }))
}

function autoReply(conversation, marker, baselineIds) {
  const messages = conversation.messages ?? []
  const report = messages.findIndex((message) => message.id.startsWith('subagent-result-') && message.content.includes(marker))
  return report >= 0 && messages.slice(report + 1).some((message) =>
    message.role === 'assistant' && !baselineIds.has(message.id) &&
    !message.id.startsWith('subagent-') && message.content.includes(marker) &&
    message.stream_outcome === 'completed')
}

if (options['list'] || options.help) {
  console.log(scenarios.join('\n'))
  console.log('node scripts/probe-subagents.mjs [scenario substring ...] [--probe-dir PATH] [--provider ID] [--model NAME]')
  console.log('Defaults use Kivio Dev and its configured chat provider/model. --self-test does not contact the app.')
  process.exit(0)
}
if (options['self-test']) {
  assert.deepEqual(journal('{"role":"user"}\n{"partial":'), [{ role: 'user' }])
  assert.throws(() => journal('{broken}\n'), SyntaxError)
  assert.deepEqual(calls([{ tool_calls: [{ function: { name: 'native__wait', arguments: '{}' } }] }])[0].args, {})
  assert.equal(calls([{ tool_calls: [{ name: 'native__task', arguments: '{"tasks":[]}' }] }])[0].name, 'task')
  assert.equal(calls([{ tool_calls: [{ function: { name: 'eval__stamp', arguments: '{"value":"probe"}' } }] }])[0].name, 'stamp')
  assert.throws(() => calls([{ tool_calls: [{ name: 'task', arguments: '{bad}' }] }]), SyntaxError)
  const report = { id: 'subagent-result-run', role: 'assistant', content: 'secret' }
  const reply = { id: 'msg-auto', role: 'assistant', content: 'secret', stream_outcome: 'completed' }
  assert.equal(autoReply({ messages: [report] }, 'secret', new Set()), false)
  assert.equal(autoReply({ messages: [reply, report] }, 'secret', new Set()), false)
  assert.equal(autoReply({ messages: [report, reply] }, 'secret', new Set(['msg-auto'])), false)
  assert.equal(autoReply({ messages: [report, reply] }, 'secret', new Set()), true)
  console.log('PASS probe parser and automatic continuation assertions')
  process.exit(0)
}

const selected = scenarios.filter((name) => !filters.length || filters.some((filter) => name.includes(filter)))
assert(selected.length, `No matching scenarios. Use --list. Received ${filters.join(', ')}`)
const configRoot = process.platform === 'win32'
  ? process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming')
  : process.platform === 'darwin' ? path.join(os.homedir(), 'Library', 'Application Support')
    : process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), '.config')
const probeDir = path.resolve(options['probe-dir'] ?? process.env.KIVIO_PROBE_DIR ?? path.join(configRoot, 'com.zmair.kivio.dev', 'chat_probe'))
const dataDir = path.dirname(probeDir)
const runId = `${new Date().toISOString().replace(/[:.]/g, '-')}-${randomUUID().slice(0, 8)}`
const evidenceDir = path.join(repo, '.scratch', 'omp-completion', 'smoke', runId)
const workdir = fs.mkdtempSync(path.join(os.tmpdir(), 'kivio-subagents-'))
fs.mkdirSync(evidenceDir, { recursive: true })
const summary = { runId, probeDir, workdir, evidenceDir, scenarios: [], limitations: [
  'Prompt-driven real-provider smoke, not deterministic tool injection.',
  'Does not verify Hub UI, in-memory registry, parking, kill, or restart recovery.',
  'Cancellation covers the parent generation and its ability to resume.',
] }
const save = (name, value) => fs.writeFileSync(path.join(evidenceDir, name), JSON.stringify(value, null, 2))
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function until(read, timeoutMs, description) {
  const deadline = Date.now() + timeoutMs
  do {
    const value = read()
    if (value) return value
    await sleep(250)
  } while (Date.now() < deadline)
  throw new Error(`Timeout after ${timeoutMs}ms waiting for ${description}`)
}
function json(file) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')) }
  catch (error) {
    if (error.code === 'ENOENT' || error instanceof SyntaxError) return null
    throw error
  }
}
let sequence = 0
const created = new Set()
async function turn(name, request) {
  const id = `omp-${runId}-${++sequence}-${name}`
  const payload = { id, cwd: workdir, ...request }
  for (const key of ['provider', 'model']) if (options[key]) payload[key] = options[key]
  const requestPath = path.join(probeDir, 'request.json')
  assert(!fs.existsSync(requestPath), 'Another probe request is pending; refusing to overwrite it')
  save(`${id}.request.json`, payload)
  const temporary = path.join(probeDir, `${id}.tmp`)
  fs.writeFileSync(temporary, JSON.stringify(payload))
  fs.renameSync(temporary, requestPath)
  try {
    await until(() => !fs.existsSync(requestPath), 15_000, 'the running debug app to consume request.json')
  } catch (error) {
    if (json(requestPath)?.id === id) fs.rmSync(requestPath)
    throw error
  }
  const result = await until(() => json(path.join(probeDir, `result-${id}.json`)), 390_000, `probe ${id}`)
  assert.equal(result.id, id)
  save(`${id}.result.json`, result)
  if (result.conversationId) created.add(result.conversationId)
  assert(!result.error, `Probe failed. ${result.error}`)
  assert(result.conversationId, 'Probe did not return a conversation ID')
  console.log(`${name} ${result.streamOutcome ?? 'no outcome'} ${result.durationMs}ms`)
  return result
}
function conversation(id) {
  return json(path.join(dataDir, 'conversations', `${id}.json`))
}
function workers(id) {
  const dir = path.join(dataDir, 'subagents', id)
  if (!fs.existsSync(dir)) return []
  return fs.readdirSync(dir).filter((name) => name.endsWith('.jsonl')).map((name) => {
    const entries = journal(fs.readFileSync(path.join(dir, name), 'utf8'))
    const init = entries.find((entry) => entry.type === 'session_init')
    const end = entries.findLast((entry) => entry.type === 'turn_end')
    return { entries, init, end, output: end?.outputFile && fs.existsSync(end.outputFile) ? fs.readFileSync(end.outputFile, 'utf8') : '' }
  })
}
async function completedWorkers(id, count) {
  return until(() => {
    const result = workers(id)
    return result.length >= count && result.every((worker) => worker.end) ? result : null
  }, 240_000, `${count} saved worker completions for ${id}`)
}
function requireCall(messages, name, predicate = () => true) {
  const call = calls(messages).find((item) => item.name === name && predicate(item))
  assert(call, `Missing actual ${name} tool call matching scenario requirements. Saw ${calls(messages).map((item) => `${item.name}/${item.status ?? 'journal'}`).join(', ')}`)
  return call
}
function completed(result) { assert.equal(result.streamOutcome, 'completed', 'Generation must complete') }
const fixtures = {}
for (const name of ['alpha', 'beta', 'gamma', 'idle', 'nested', 'recovery', 'eval', 'pool']) {
  fixtures[name] = `OMP-${name}-${randomUUID()}`
  fs.writeFileSync(path.join(workdir, `${name}.txt`), fixtures[name])
}
fs.writeFileSync(path.join(workdir, 'delayed.cjs'), "setTimeout(() => process.stdout.write(require('node:fs').readFileSync('idle.txt','utf8')), 20000)\n")
const roles = path.join(workdir, '.kivio', 'agents')
fs.mkdirSync(roles, { recursive: true })
const role = (name, frontmatter, body) => fs.writeFileSync(path.join(roles, `${name}.md`),
  `---\nname: ${name}\ndescription: Local smoke fixture role\n${frontmatter}\n---\n${body}\n`)
role('probe-delay', 'tools: [bash]\nspawns: false', 'Follow the assignment exactly. Run node delayed.cjs with bash once. Its delay is intentional. Return its exact output.')
role('probe-peer', 'tools: [read, write, wait]\nspawns: false', 'Follow the assigned IRC exchange exactly. Do not finish until you receive the requested reply.')
role('probe-parent', 'tools: [task, wait, read, write]\nspawns: [probe-peer]', 'Follow the assignment exactly. Delegate the requested nested peer and reply through write agent:// URLs. Do not invent file contents.')
role('probe-eval', 'tools: []\nspawns: false', 'Call the assigned shared custom tool exactly once. Submit its exact returned object with yield data. Do not invent its nonce. Do not call any other tool except yield.')
role('probe-pool', 'tools: []\nspawns: false', 'For each assigned workpool item, call the shared echo custom tool exactly once with its requested value. Submit its exact returned object with yield using the batch-local numeric key. Do not invent the nonce. The last item ends the turn automatically.')

async function runScenario(name) {
  if (name === 'batch-scouts') {
    const tasks = ['alpha', 'beta', 'gamma'].map((file) => ({ name: `Smoke-${file}`, agent: 'scout', solutionSpace: 'Read only the assigned fixture.', task: `Read ${file}.txt with the read tool. Return its exact contents. Do not write files or delegate.` }))
    const result = await turn(name, { prompt: `Execute this single batch with task using these exact arguments ${JSON.stringify({ context: 'Independent read-only fixture checks.', tasks })}. Then call wait with {} until all three finish. Read each agent:// output if necessary. Finish with all three file contents. Do not read the files yourself.` })
    completed(result)
    const saved = conversation(result.conversationId)
    assert(saved, 'Conversation must be persisted')
    requireCall(saved.messages, 'task', (call) => call.args.tasks?.length === 3 && call.args.tasks.every((task) => task.agent === 'scout'))
    requireCall(saved.messages, 'wait', (call) => Object.keys(call.args).length === 0)
    const children = await completedWorkers(result.conversationId, 3)
    assert.equal(children.length, 3)
    assert.equal(new Set(children.map((child) => child.init.id)).size, 3)
    for (const file of ['alpha', 'beta', 'gamma']) {
      const child = children.find((item) => item.init.task.includes(`${file}.txt`))
      assert(child, `Missing worker for ${file}`)
      assert.equal(child.init.depth, 1)
      assert.equal(child.init.agentDefinition.id, 'scout')
      assert.equal(child.end.progress.status, 'completed')
      assert(child.output.includes(fixtures[file]), `${file} output must contain its independent file nonce`)
      assert(result.answer.includes(fixtures[file]), `Parent must consume the ${file} result`)
      requireCall(child.entries, 'read', (call) => JSON.stringify(call.args).includes(`${file}.txt`))
      assert(!child.init.tools.some((tool) => /^(native__)?(write|edit|bash|task)$/.test(tool)), 'Scout must retain its read-only tool capability')
    }
    return { conversationId: result.conversationId, agents: children.map((child) => child.init.id) }
  }
  if (name === 'idle-auto-result') {
    const result = await turn(name, { prompt: 'Call task once with {"context":"Delayed output tests idle automatic result delivery.","tasks":[{"name":"Smoke-delayed","agent":"probe-delay","solutionSpace":"Run the fixture command only.","task":"Run node delayed.cjs once with bash and return its exact output. The intentional delay must finish."}]}. Do not wait, read files, call other tools, or continue working. Immediately finish this first turn with only DISPATCHED. When its result arrives automatically in a later turn, reply with exactly the child output.' })
    completed(result)
    assert(!(result.toolCalls ?? []).some((call) => /(^|__)wait$/.test(call.name)), 'Initial parent must become idle without waiting')
    const baseline = conversation(result.conversationId)
    requireCall(baseline.messages, 'task')
    assert(!baseline.messages.some((message) => message.id.startsWith('subagent-result-')), 'Child finished before parent became idle; this run does not prove idle continuation')
    const ids = new Set(baseline.messages.map((message) => message.id))
    const saved = await until(() => {
      const current = conversation(result.conversationId)
      return current && autoReply(current, fixtures.idle, ids) ? current : null
    }, 240_000, 'new completed assistant reply after automatic child report without another user request')
    assert.equal(saved.messages.filter((message) => message.role === 'user').length, 1, 'No second user turn may trigger the result')
    const [child] = await completedWorkers(result.conversationId, 1)
    requireCall(child.entries, 'bash', (call) => JSON.stringify(call.args).includes('delayed.cjs'))
    assert(child.output.includes(fixtures.idle), 'Child must obtain the private nonce from its delayed command')
    return { conversationId: result.conversationId, agent: child.init.id }
  }
  if (name === 'nested-irc') {
    const assignment = 'Use task with nonempty context and solutionSpace to spawn exactly one probe-peer named Smoke-peer. Its full ID will be Smoke-parent.Smoke-peer. Assign it: Send write path agent://Smoke-parent content "Please read nested.txt and send its exact contents back". Then call wait {} until that reply arrives. Finish with that exact reply. You, Smoke-parent, must wait for the question, read nested.txt yourself, send its exact contents via write path agent://Smoke-parent.Smoke-peer, then wait for the peer to finish and return its exact final output. Do not put file contents in the initial task.'
    const result = await turn(name, { prompt: `Call task with ${JSON.stringify({ context: 'Nested delegation and a two-way IRC question/reply.', tasks: [{ name: 'Smoke-parent', agent: 'probe-parent', solutionSpace: 'Use the specified peer and IRC exchange only.', task: assignment }] })}. Then call wait {} until the entire nested delegation and IRC exchange finish. Return the final output.` })
    completed(result)
    const children = await completedWorkers(result.conversationId, 2)
    const parent = children.find((child) => child.init.depth === 1)
    const peer = children.find((child) => child.init.depth === 2)
    assert(parent && peer, 'Saved contracts must prove two delegation depths')
    assert.equal(peer.init.parentId, parent.init.id)
    assert.equal(parent.init.agentDefinition.source, 'project')
    requireCall(parent.entries, 'task', (call) => call.args.tasks?.some((task) => task.agent === 'probe-peer'))
    requireCall(peer.entries, 'write', (call) => call.args.path === `agent://${parent.init.id}`)
    requireCall(parent.entries, 'read', (call) => JSON.stringify(call.args).includes('nested.txt'))
    requireCall(parent.entries, 'write', (call) => call.args.path === `agent://${peer.init.id}` && call.args.content.includes(fixtures.nested))
    requireCall(peer.entries, 'wait', (call) => Object.keys(call.args).length === 0)
    assert(peer.output.includes(fixtures.nested), 'Peer must return the private nonce obtained from the IRC reply')
    assert(parent.output.includes(fixtures.nested), 'Nested parent must consume the peer result')
    assert.equal(peer.init.spawns, '', 'Peer cannot recursively spawn')
    assert(!peer.init.tools.some((tool) => /^(native__)?(task|bash|edit)$/.test(tool)), 'Peer must retain its limited role tools')
    return { conversationId: result.conversationId, parent: parent.init.id, peer: peer.init.id }
  }
  if (name === 'task-error') {
    const result = await turn(name, { prompt: 'Call task exactly once with {"context":"Deliberate unknown role error recovery test.","tasks":[{"agent":"probe-role-does-not-exist","solutionSpace":"No work should execute.","task":"return ok"}]}. This deliberately invalid role tests the error path. After the tool error, read recovery.txt and reply with its exact contents. Do not substitute an existing agent.' })
    completed(result)
    const saved = conversation(result.conversationId)
    requireCall(saved.messages, 'task', (call) => call.args.tasks?.some((task) => task.agent === 'probe-role-does-not-exist') && call.status === 'error')
    requireCall(saved.messages, 'read', (call) => JSON.stringify(call.args).includes('recovery.txt') && call.status === 'success')
    assert(result.answer.includes(fixtures.recovery), 'Parent must recover after the task error')
    return { conversationId: result.conversationId }
  }
  if (name === 'eval-shared') {
    const code = `(async () => {
      tool(({value}) => ({value, nonce: require('node:fs').readFileSync('eval.txt', 'utf8')}), {
        name: 'stamp', description: 'Return the provided value and a private fixture nonce.',
        parameters: {type:'object',properties:{value:{type:'string'}},required:['value'],additionalProperties:false}
      });
      return await agent({prompt:'Call the shared stamp tool exactly once with value probe. Then yield data containing exactly its returned object.',agent:'probe-eval',label:'Smoke-eval',tools:['stamp'],
        schema:{type:'object',properties:{value:{type:'string'},nonce:{type:'string'}},required:['value','nonce'],additionalProperties:false},schemaMode:'strict'});
    })()`
    const result = await turn(name, { prompt: `Call eval exactly once with language js, timeout 120, and this exact code. It registers a shared tool and returns a background agent handle. Then use wait {} until the child completes. Finish with its returned nonce. Do not read fixture files yourself.\n${code}` })
    completed(result)
    const saved = conversation(result.conversationId)
    const evalCall = requireCall(saved.messages, 'eval', (call) => call.status === 'success' && call.args.code.includes("name: 'stamp'"))
    const handle = JSON.parse(evalCall.result_preview)
    assert.equal(handle.agent, 'probe-eval')
    assert.equal(typeof handle.id, 'string', 'Eval agent must return a background job handle')
    requireCall(saved.messages, 'wait', (call) => Object.keys(call.args).length === 0)
    const children = await completedWorkers(result.conversationId, 1)
    assert.equal(children.length, 1)
    const [child] = children
    assert.equal(handle.agentId, child.init.id)
    assert.equal(child.init.agentDefinition.id, 'probe-eval')
    assert.deepEqual(child.init.agentDefinition.tools, [])
    assert(child.init.tools.includes('eval__stamp'), 'Shared eval tool must be mounted despite the empty builtin list')
    assert(!child.init.tools.some((tool) => /^(native__)?(read|write|bash|task)$/.test(tool)), 'Fixture role must retain its empty builtin grant')
    const stamps = calls(child.entries).filter((call) => call.name === 'stamp')
    assert.equal(stamps.length, 1, 'Child must really invoke the shared tool exactly once')
    assert.equal(stamps[0].args.value, 'probe')
    assert.equal(child.end.progress.status, 'completed')
    assert.deepEqual(child.end.progress.structuredOutput.data, { value: 'probe', nonce: fixtures.eval })
    assert(result.answer.includes(fixtures.eval), 'Parent must receive the actual shared tool result')
    return { conversationId: result.conversationId, agent: child.init.id, jobId: handle.id }
  }
  if (name === 'workpool-reuse') {
    const code = `(async () => {
      tool(async ({value}) => {
        if (value === 'slow') await new Promise(resolve => setTimeout(resolve, 20000));
        return {value, nonce: require('node:fs').readFileSync('pool.txt', 'utf8')};
      }, {name:'echo',description:'Return a fixture nonce and the value. Only slow intentionally waits twenty seconds.',
        parameters:{type:'object',properties:{value:{type:'string'}},required:['value'],additionalProperties:false}});
      const pool = await workpool({name:'Smoke-pool',agent:'probe-pool',tools:['echo'],context:'For each item call echo exactly once with its requested value, then yield the exact returned object with the batch-local numeric key.'});
      await pool.push('Call echo with value fast.', 'Call echo with value slow.');
      const deadline = Date.now() + 70000;
      let before;
      for (;;) {
        before = await pool.status();
        if (before.freshAgents) throw new Error('This reuse scenario requires the existing freshAgents setting to be false.');
        if (before.items[0].status === 'completed' && before.items[1].status === 'running') break;
        if (before.closed || Date.now() > deadline) throw new Error('Could not observe an idle fast worker while the slow worker remained active.');
        await new Promise(resolve => setTimeout(resolve, 250));
      }
      await pool.push('Call echo with value later.');
      const drained = await pool.drain();
      return {before, status:await pool.status(), outcomes:drained.batches.map(batch => ({items:batch.items,status:batch.status,data:batch.output.data}))};
    })()`
    const result = await turn(name, { prompt: `Use eval exactly once with language js, timeout 120, and this exact code. It deliberately overlaps a slow worker with a later push to prove reuse. Do not change the code or read fixture files yourself. After it finishes, report its status and returned nonce.\n${code}` })
    completed(result)
    const saved = conversation(result.conversationId)
    const evalCall = requireCall(saved.messages, 'eval', (call) => call.status === 'success' && call.args.code.includes("name:'Smoke-pool'"))
    const pool = JSON.parse(evalCall.result_preview)
    save(`${name}.pool-result.json`, pool)
    assert.equal(pool.before.items[0].status, 'completed')
    assert.equal(pool.before.items[1].status, 'running')
    assert.equal(pool.status.freshAgents, false)
    assert.equal(pool.status.items.length, 3)
    assert(pool.status.items.every((item) => item.status === 'completed'), 'Every real pool item must complete')
    assert.equal(pool.status.agents.length, 2, 'Later push must reuse one of the first two workers')
    assert(pool.status.agents.some((agent) => agent.turns === 2), 'A retained worker must serve two turns')
    const returned = pool.outcomes.flatMap((batch) => Object.values(batch.data))
    assert.deepEqual(returned.map((item) => item.value).sort(), ['fast', 'later', 'slow'])
    assert(returned.every((item) => item.nonce === fixtures.pool), 'Per-item JSON must contain the private tool nonce')
    const children = await completedWorkers(result.conversationId, 2)
    assert.equal(children.length, 2)
    const echoes = children.flatMap((child) => calls(child.entries)).filter((call) => call.name === 'echo')
    assert.deepEqual(echoes.map((call) => call.args.value).sort(), ['fast', 'later', 'slow'], 'Exactly three real shared-tool calls must serve the three items')
    assert(children.some((child) => child.entries.filter((entry) => entry.type === 'turn_start').length === 2), 'One saved identity must contain two distinct worker turns')
    for (const child of children) {
      assert.equal(child.init.agentDefinition.id, 'probe-pool')
      assert.deepEqual(child.init.agentDefinition.tools, [])
      assert(child.init.tools.includes('eval__echo'))
      assert.equal(child.end.progress.status, 'completed')
    }
    return { conversationId: result.conversationId, agents: children.map((child) => child.init.id), outcomes: returned }
  }
  const cancelled = await turn(name, { prompt: 'Call bash to execute node delayed.cjs and wait for its output. Do not answer before the command finishes.', cancelAfterMs: 1500 })
  assert.equal(cancelled.streamOutcome, 'cancelled', 'Cancellation must use the cancelled outcome')
  const resumed = await turn(name, { conversationId: cancelled.conversationId, prompt: 'Read recovery.txt using read and reply with exactly its contents.' })
  completed(resumed)
  requireCall(conversation(resumed.conversationId).messages, 'read', (call) => JSON.stringify(call.args).includes('recovery.txt') && call.status === 'success')
  assert(resumed.answer.includes(fixtures.recovery), 'Same conversation must resume after cancellation')
  return { conversationId: resumed.conversationId }
}

function preserve() {
  for (const id of created) {
    const file = path.join(dataDir, 'conversations', `${id}.json`)
    if (fs.existsSync(file)) fs.copyFileSync(file, path.join(evidenceDir, `${id}.conversation.json`))
    const source = path.join(dataDir, 'subagents', id)
    if (fs.existsSync(source)) fs.cpSync(source, path.join(evidenceDir, id), { recursive: true })
  }
  save('summary.json', summary)
}
try {
  assert(fs.existsSync(probeDir), `Probe unavailable at ${probeDir}. Start the debug app before running this script.`)
  for (const name of selected) {
    console.log(`RUN ${name}`)
    try {
      const evidence = await runScenario(name)
      summary.scenarios.push({ name, status: 'pass', ...evidence })
      console.log(`PASS ${name}`)
    } catch (error) {
      summary.scenarios.push({ name, status: 'fail', detail: error.message })
      console.error(`FAIL ${name}: ${error.message}`)
    }
    preserve()
  }
} catch (error) {
  summary.error = error.message
  for (const name of selected) summary.scenarios.push({ name, status: 'skip', detail: error.message })
  console.error(`SKIP ${error.message}`)
} finally {
  preserve()
  console.log(`Evidence ${path.join(evidenceDir, 'summary.json')}`)
  process.exitCode = summary.scenarios.every((scenario) => scenario.status === 'pass') ? 0 : 1
}
