import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import test from 'node:test'
import kivioMcpExtension from '../src-tauri/resources/pi/kivio-pi-mcp.mjs'

async function fixture(t) {
  const calls = []
  const cancelled = Promise.withResolvers()
  const started = Promise.withResolvers()
  const catalog = ['html_render', 'html_preview'].map(name => ({
    name, description: name, inputSchema: { type: 'object', properties: { html: { type: 'string' } } },
  }))
  const state = { catalog, revoked: false }
  const server = createServer(async (request, response) => {
    const chunks = []
    for await (const chunk of request) chunks.push(chunk)
    const message = JSON.parse(Buffer.concat(chunks).toString())
    calls.push({ ...message, authorization: request.headers.authorization })
    if (state.revoked) { response.writeHead(401).end(); return }
    assert.equal(request.headers['mcp-protocol-version'], '2025-11-25')
    let result
    if (message.method === 'initialize') {
      result = { protocolVersion: '2025-11-25', capabilities: { tools: {} }, serverInfo: { name: 'Kivio-server', version: '1' } }
      response.setHeader('mcp-session-id', 'test-session')
    } else {
      assert.equal(request.headers['mcp-session-id'], 'test-session')
      if (message.method === 'notifications/cancelled') {
        cancelled.resolve(message.params.requestId)
      } else if (message.method === 'tools/list') {
        result = { tools: state.catalog }
      } else if (message.method === 'tools/call') {
        const args = message.params.arguments
        if (args.html === 'wait') { started.resolve(message.id); return }
        if (args.html === 'error') result = { isError: true, content: [{ type: 'text', text: 'Invalid HTML' }] }
        else result = { content: [{ type: 'text', text: 'htmlrender-test.html' },
          { type: 'image', mimeType: 'image/png', data: 'cG5n' }], structuredContent: { height: 120 } }
      }
    }
    if (message.id === undefined) { response.writeHead(202).end(); return }
    response.setHeader('Content-Type', 'application/json')
    response.end(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }))
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => { server.closeAllConnections(); server.close() })
  const endpoint = `http://127.0.0.1:${server.address().port}/mcp`
  const load = async (authorization = 'Bearer conversation-a') => {
    const tools = new Map()
    const events = new Map()
    const previous = { url: process.env.KIVIO_MCP_URL, authorization: process.env.KIVIO_MCP_AUTHORIZATION }
    process.env.KIVIO_MCP_URL = endpoint
    process.env.KIVIO_MCP_AUTHORIZATION = authorization
    try {
      await kivioMcpExtension({ registerTool: tool => tools.set(tool.name, tool), on: (event, handler) => events.set(event, handler) })
      assert.equal(process.env.KIVIO_MCP_AUTHORIZATION, authorization)
    } finally {
      for (const [key, value] of [['KIVIO_MCP_URL', previous.url], ['KIVIO_MCP_AUTHORIZATION', previous.authorization]]) {
        if (value === undefined) delete process.env[key]
        else process.env[key] = value
      }
    }
    return { tools, events, execute: (html, signal) => tools.get('mcp__Kivio-server__html_render').execute('call', { html }, signal) }
  }
  return { state, calls, started: started.promise, cancelled: cancelled.promise, load }
}

test('Pi discovers chat tools and preserves text, images and structured output', async t => {
  const { load, calls } = await fixture(t)
  const client = await load()
  assert.deepEqual([...client.tools.keys()], ['mcp__Kivio-server__html_render', 'mcp__Kivio-server__html_preview'])
  const result = await client.execute('<p>Hello</p>')
  assert.deepEqual(result.content, [{ type: 'text', text: 'htmlrender-test.html' }, { type: 'image', mimeType: 'image/png', data: 'cG5n' }])
  assert.equal(result.structuredContent.structuredContent.height, 120)
  assert.ok(calls.every(call => call.authorization === 'Bearer conversation-a'))
  assert.deepEqual(calls.at(-1).params, { name: 'html_render', arguments: { html: '<p>Hello</p>' } })
  await assert.rejects(client.execute('error'), /Invalid HTML/)
})

test('Pi keeps credentials separate and cannot call after revocation', async t => {
  const { load, calls, state } = await fixture(t)
  const a = await load('Bearer a')
  const b = await load('Bearer b')
  await a.execute('a')
  await b.execute('b')
  assert.deepEqual(calls.filter(call => call.method === 'tools/call').map(call => call.authorization), ['Bearer a', 'Bearer b'])
  state.revoked = true
  await assert.rejects(a.execute('late'), /failed \(401\)/)
})

test('Pi cancellation notifies MCP and shutdown prevents further requests', async t => {
  const { load, started, cancelled, calls } = await fixture(t)
  const client = await load()
  const controller = new AbortController()
  const pending = client.execute('wait', controller.signal)
  const rejected = assert.rejects(pending, /abort/i)
  const id = await started
  controller.abort()
  await rejected
  assert.equal(await cancelled, id)
  client.events.get('session_shutdown')()
  const count = calls.length
  await assert.rejects(client.execute('late'), /abort/i)
  assert.equal(calls.length, count)
})

test('Pi rejects non-chat catalogs', async t => {
  const { load, state } = await fixture(t)
  state.catalog[0].name = 'kivio_send_message'
  await assert.rejects(load(), /Invalid Kivio-server chat tool/)
})
