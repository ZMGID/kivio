#!/usr/bin/env node
// Run against an already running debug app with a chat or main WebView open.
import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const options = {}
for (let index = 2; index < process.argv.length; index++) {
  const option = process.argv[index]
  if (['--probe-dir', '--provider', '--model', '--app-pid', '--timeout-seconds'].includes(option)) {
    const value = process.argv[++index]
    assert(value && !value.startsWith('--'), `${option} requires a value`)
    options[option.slice(2)] = value
  } else {
    assert(['--help', '--self-test'].includes(option), `Unknown option ${option}`)
    options[option.slice(2)] = true
  }
}

function readJson(file) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')) }
  catch (error) {
    if (error.code === 'ENOENT') return null
    throw error
  }
}

function publishRequest(directory, request) {
  const temporary = path.join(directory, `${request.id}.tmp`)
  fs.writeFileSync(temporary, JSON.stringify(request), { flag: 'wx' })
  try {
    // A hard link publishes a complete file and refuses to replace a pending request.
    fs.linkSync(temporary, path.join(directory, 'request.json'))
  } finally {
    fs.unlinkSync(temporary)
  }
}

function assertSavedTurn(conversation, prompt, result, userCount) {
  assert(conversation, 'The conversation was not persisted')
  assert.equal(conversation.id, result.conversationId)
  assert.notEqual(conversation.agent_runtime?.kind, 'external', 'Select a built-in default runtime for this stack regression')
  const users = conversation.messages.filter((message) => message.role === 'user')
  assert.equal(users.length, userCount, 'Each IPC send must persist exactly one user message')
  assert.equal(users.at(-1).content, prompt)
  const userIndex = conversation.messages.findLastIndex((message) => message.role === 'user')
  const assistants = conversation.messages.slice(userIndex + 1).filter((message) => message.role === 'assistant')
  assert.equal(assistants.length, 1, 'The current user message must have one persisted assistant')
  assert.equal(assistants[0].stream_outcome, result.streamOutcome)
  assert.equal(assistants[0].content, result.answer)
}

if (options.help) {
  console.log('node scripts/probe-chat-send.mjs [--probe-dir PATH] [--provider ID] [--model NAME] [--app-pid PID] [--timeout-seconds 360]')
  console.log('Checks two completed IPC sends, cancellation, and another completed send in one built-in conversation.')
  console.log('Uses KIVIO_PROBE_DIR or the default Kivio Dev app-data directory. --app-pid detects app exit immediately.')
  process.exit(0)
}

if (options['self-test']) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kivio-send-self-test-'))
  try {
    publishRequest(directory, { id: 'first', prompt: 'hello' })
    assert.throws(() => publishRequest(directory, { id: 'second', prompt: 'other' }), { code: 'EEXIST' })
    assert.equal(readJson(path.join(directory, 'request.json')).id, 'first')
    const result = { conversationId: 'conversation', streamOutcome: 'completed', answer: 'ok' }
    const conversation = { id: 'conversation', messages: [
      { role: 'user', content: 'hello' },
      { role: 'assistant', content: 'ok', stream_outcome: 'completed' },
    ] }
    assertSavedTurn(conversation, 'hello', result, 1)
    assert.throws(() => assertSavedTurn({ ...conversation, messages: conversation.messages.slice(0, 1) }, 'hello', result, 1))
    assert.throws(() => assertSavedTurn({ ...conversation, agent_runtime: { kind: 'external' } }, 'hello', result, 1))
    console.log('PASS request publication and persisted-turn assertions')
  } finally {
    fs.rmSync(directory, { recursive: true, force: true })
  }
  process.exit(0)
}

const timeoutSeconds = Number(options['timeout-seconds'] ?? 360)
assert(Number.isSafeInteger(timeoutSeconds) && timeoutSeconds >= 1 && timeoutSeconds <= 3600, '--timeout-seconds must be between 1 and 3600')
const appPid = options['app-pid'] === undefined ? null : Number(options['app-pid'])
assert(appPid === null || (Number.isSafeInteger(appPid) && appPid > 0), '--app-pid must be a positive process ID')
const configRoot = process.platform === 'win32'
  ? process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming')
  : process.platform === 'darwin' ? path.join(os.homedir(), 'Library', 'Application Support')
    : process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), '.config')
const probeDir = path.resolve(options['probe-dir'] ?? process.env.KIVIO_PROBE_DIR ?? path.join(configRoot, 'com.zmair.kivio.dev', 'chat_probe'))
const dataDir = path.dirname(probeDir)
const runId = `send-${Date.now()}-${randomUUID().slice(0, 8)}`
const workdir = fs.mkdtempSync(path.join(os.tmpdir(), 'kivio-chat-send-'))
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function until(read, timeoutMs, description) {
  const deadline = Date.now() + timeoutMs
  do {
    if (appPid !== null) {
      try { process.kill(appPid, 0) }
      catch { throw new Error(`App process ${appPid} exited while waiting for ${description}`) }
    }
    const value = read()
    if (value) return value
    await sleep(200)
  } while (Date.now() < deadline)
  throw new Error(`Timeout after ${timeoutMs}ms waiting for ${description}; the debug app may have exited`)
}

let conversationId
let sequence = 0
async function turn(name, prompt, expectedOutcome, cancelAfterMs) {
  const id = `${runId}-${++sequence}-${name}`
  const request = {
    id, prompt, cwd: workdir, sendThroughIpc: true,
    timeoutSeconds, computeContextStats: true,
    ...(conversationId ? { conversationId } : {}),
    ...(cancelAfterMs === undefined ? {} : { cancelAfterMs }),
  }
  for (const key of ['provider', 'model']) if (options[key]) request[key] = options[key]
  publishRequest(probeDir, request)
  await until(() => !fs.existsSync(path.join(probeDir, 'request.json')), 15_000, `request ${id} consumption`)
  const result = await until(() => readJson(path.join(probeDir, `result-${id}.json`)), timeoutSeconds * 1000 + 15_000, `IPC result ${id}`)
  assert.equal(result.id, id)
  assert(!result.error, `Probe failed for ${name}; inspect result-${id}.json`)
  assert(result.conversationId, 'The probe did not return a conversation ID')
  if (conversationId) assert.equal(result.conversationId, conversationId, 'Every send must use the same conversation')
  conversationId = result.conversationId
  assert.equal(result.streamOutcome, expectedOutcome, `Unexpected outcome for ${name}`)
  if (expectedOutcome === 'completed') assert(result.answer.trim(), 'A completed turn must have an answer')
  const conversation = readJson(path.join(dataDir, 'conversations', `${conversationId}.json`))
  assertSavedTurn(conversation, prompt, result, sequence)
  console.log(`PASS ${name} ${expectedOutcome} ${result.durationMs}ms`)
}

try {
  await turn('first', `Reply with the short text IPC first ${runId}. Do not use tools.`, 'completed')
  await turn('continued', `Reply with the short text IPC continued ${runId}. Do not use tools.`, 'completed')
  await turn('cancelled', 'Write every integer from 1 to 10000 on separate lines. Do not use tools.', 'cancelled', 50)
  await turn('resumed', `Reply with the short text IPC resumed ${runId}. Do not use tools.`, 'completed')
  console.log(`PASS four production IPC sends in ${conversationId}`)
} finally {
  console.log(`Probe results retained in ${probeDir}; fixture directory retained at ${workdir}`)
}
