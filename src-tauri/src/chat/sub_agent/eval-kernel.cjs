const { createInterface } = require('node:readline')
const { PassThrough } = require('node:stream')
const repl = require('node:repl')
const { inspect } = require('node:util')

const send = value => process.stdout.write(`${JSON.stringify(value)}\n`)
let sequence = 0
const pending = new Map()
const tools = new Map()
let currentCell = null
const host = (method, args) => new Promise((resolve, reject) => {
  const id = `host-${++sequence}`
  pending.set(id, { resolve, reject })
  send({ type: 'host', id, cell: currentCell, method, args })
})
const server = repl.start({ prompt: '', input: new PassThrough(), output: new PassThrough(), terminal: false, useGlobal: false })
Object.assign(server.context, {
  console: Object.fromEntries(['log', 'info', 'warn', 'error'].map(name => [name, (...args) => send({ type: 'output', cell: currentCell, text: args.map(value => typeof value === 'string' ? value : inspect(value)).join(' ') })])),
  tool(fn, descriptor = {}) {
    const name = descriptor.name || fn.name
    if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(name) || typeof fn !== 'function') throw new Error('A tool needs a valid name and function')
    tools.set(name, { fn, name, description: descriptor.description || '', parameters: descriptor.parameters || { type: 'object', properties: {} }, language: 'js' })
    return fn
  },
  undefineTool(name) { return tools.delete(name) },
  agent(request) { return host('agent', request) },
  async workpool(options) {
    const { id } = await host('pool.create', options)
    return {
      id,
      push: (...items) => host('pool.push', { id, items: items.flat() }),
      peek: () => host('pool.peek', { id }),
      status: () => host('pool.status', { id }),
      drain: () => host('pool.drain', { id }),
      close: () => host('pool.close', { id }),
    }
  },
})

let cells = Promise.resolve()
async function execute(request) {
  try {
    let value
    if (request.op === 'eval') {
      currentCell = request.id
      value = await new Promise((resolve, reject) => server.eval(request.code + '\n', server.context, 'subagent-eval', (error, value) => error ? reject(error) : resolve(value)))
      currentCell = null
    } else if (request.op === 'describe') {
      const names = request.names || []
      value = [...tools.values()].filter(tool => !names.length || names.includes(tool.name)).map(({ fn, ...descriptor }) => descriptor)
    } else if (request.op === 'invoke') {
      const entry = tools.get(request.name)
      if (!entry) throw new Error(`Tool ${request.name} is unavailable`)
      value = await entry.fn(request.arguments || {})
    } else throw new Error('Unknown kernel operation')
    try { send({ type: 'result', id: request.id, ok: true, value: value === undefined ? null : value }) }
    catch { send({ type: 'result', id: request.id, ok: true, value: inspect(value) }) }
  } catch (error) {
    currentCell = null
    send({ type: 'result', id: request.id, ok: false, error: error.message || String(error) })
  }
}

createInterface({ input: process.stdin, terminal: false }).on('line', line => {
  let request
  try { request = JSON.parse(line) } catch { return }
  if (request.type === 'host-result') {
    const waiter = pending.get(request.id)
    if (!waiter) return
    pending.delete(request.id)
    if (request.ok) waiter.resolve(request.value)
    else waiter.reject(new Error(request.error || 'Host call failed'))
  } else if (request.op === 'eval') {
    cells = cells.then(() => execute(request))
  } else {
    void execute(request)
  }
}).on('close', () => process.exit(0))
