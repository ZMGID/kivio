const PROTOCOL = '2025-11-25'

export default async function kivioMcpExtension(pi) {
  const endpoint = process.env.KIVIO_MCP_URL
  const authorization = process.env.KIVIO_MCP_AUTHORIZATION
  if (!endpoint || !authorization) throw new Error('Kivio-server connection is missing')
  const url = new URL(endpoint)
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || url.pathname !== '/mcp') {
    throw new Error('Kivio-server must use the local MCP endpoint')
  }
  let nextId = 1
  let sessionId
  const shutdown = new AbortController()
  const post = async (message, signal) => {
    const response = await fetch(url, {
      method: 'POST', redirect: 'error', signal,
      headers: {
        Authorization: authorization, 'Content-Type': 'application/json',
        Accept: 'application/json, text/event-stream', 'MCP-Protocol-Version': PROTOCOL,
        ...(sessionId ? { 'MCP-Session-Id': sessionId } : {}),
      },
      body: JSON.stringify({ jsonrpc: '2.0', ...message }),
    })
    sessionId = response.headers.get('mcp-session-id') || sessionId
    if (!response.ok) throw new Error(`Kivio-server ${message.method} failed (${response.status})`)
    if (message.id === undefined) { await response.body?.cancel(); return }
    const reply = await response.json()
    if (reply.id !== message.id || reply.jsonrpc !== '2.0') throw new Error('Invalid Kivio-server response')
    if (reply.error) throw new Error(reply.error.message || 'Kivio-server request failed')
    return reply.result
  }
  const request = async (method, params, signal) => {
    const id = nextId++
    try {
      return await post({ id, method, params }, signal)
    } catch (error) {
      if (signal.aborted && !shutdown.signal.aborted) {
        await post({ method: 'notifications/cancelled', params: { requestId: id } }, AbortSignal.timeout(1000)).catch(() => {})
      }
      throw error
    }
  }
  pi.on('session_shutdown', () => shutdown.abort())
  const startupSignal = AbortSignal.any([shutdown.signal, AbortSignal.timeout(10_000)])
  await request('initialize', {
    protocolVersion: PROTOCOL, capabilities: {}, clientInfo: { name: 'kivio-pi', version: '1.0.0' },
  }, startupSignal)
  await post({ method: 'notifications/initialized' }, startupSignal)
  const catalog = await request('tools/list', {}, startupSignal)
  if (!Array.isArray(catalog?.tools)) throw new Error('Invalid Kivio-server tool catalog')
  for (const tool of catalog.tools) {
    if (!['html_render', 'html_preview'].includes(tool.name) || !tool.inputSchema || tool.inputSchema.type !== 'object') {
      throw new Error('Invalid Kivio-server chat tool')
    }
    pi.registerTool({
      name: `mcp__Kivio-server__${tool.name}`, label: tool.name,
      description: tool.description || tool.name, parameters: tool.inputSchema,
      async execute(_id, args, signal) {
        const result = await request('tools/call', { name: tool.name, arguments: args },
          AbortSignal.any([shutdown.signal, ...(signal ? [signal] : []), AbortSignal.timeout(630_000)]))
        if (!Array.isArray(result?.content)) throw new Error('Invalid Kivio-server tool result')
        const content = result.content.filter(part => part?.type === 'text' && typeof part.text === 'string'
          || part?.type === 'image' && typeof part.data === 'string' && typeof part.mimeType === 'string')
        if (result.isError) throw new Error(content.filter(part => part.type === 'text').map(part => part.text).join('\n') || 'Kivio-server tool failed')
        return { content, structuredContent: result, details: { server: 'Kivio-server', tool: tool.name } }
      },
    })
  }
}
