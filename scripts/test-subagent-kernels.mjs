import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'

const root = new URL('../src-tauri/src/chat/sub_agent/', import.meta.url)
async function verify(language, command, args) {
  const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'inherit'], windowsHide: true })
  const pending = new Map()
  let sequence = 0
  const request = data => new Promise((resolve, reject) => {
    const id = `test-${++sequence}`
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${language} timed out at ${data.op}`)) }, 10000)
    pending.set(id, message => { clearTimeout(timer); message.ok ? resolve(message.value) : reject(new Error(message.error)) })
    child.stdin.write(JSON.stringify({ id, ...data }) + '\n')
  })
  createInterface({ input: child.stdout }).on('line', async line => {
    const message = JSON.parse(line)
    if (message.type === 'result') { pending.get(message.id)?.(message); pending.delete(message.id) }
    if (message.type === 'host') {
      const result = await request({ op: 'invoke', name: 'double', arguments: { value: 21 } })
      child.stdin.write(JSON.stringify({ type: 'host-result', id: message.id, ok: true, value: result }) + '\n')
    }
  })
  try {
    assert.equal(await request({ op: 'eval', code: language === 'js' ? 'var answer = 41; answer' : 'answer = 41\nanswer' }), 41)
    assert.equal(await request({ op: 'eval', code: language === 'js' ? 'answer += 1; answer' : 'answer += 1\nanswer' }), 42)
    await request({ op: 'eval', code: language === 'js' ? 'tool(({ value }) => value * 2, { name: "double", parameters: { type: "object", properties: { value: { type: "number" } }, required: ["value"] } })' : '@tool(parameters={"type":"object","properties":{"value":{"type":"number"}},"required":["value"]})\ndef double(value):\n    return value * 2' })
    const descriptors = await request({ op: 'describe', names: ['double'] })
    assert.equal(descriptors[0].name, 'double')
    assert.equal(await request({ op: 'invoke', name: 'double', arguments: { value: 4 } }), 8)
    assert.equal(await request({ op: 'eval', code: 'await agent({})' }), 42, 'child may invoke an owner tool while owner awaits agent')
    await assert.rejects(request({ op: 'invoke', name: 'missing', arguments: {} }), /unavailable/)
    console.log(`${language} kernel passed persistence, descriptors, invocation, reentrant agent callback and failures`)
  } finally { child.kill() }
}

await verify('js', process.execPath, [fileURLToPath(new URL('eval-kernel.cjs', root))])
const python = process.env.KIVIO_TEST_PYTHON || (process.platform === 'win32' ? 'py' : 'python3')
await verify('py', python, ['-u', fileURLToPath(new URL('eval-kernel.py', root))])
