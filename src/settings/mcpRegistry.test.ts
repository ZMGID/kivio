// mcpRegistry 数据层自检：喂入各源代表性 JSON，验证归一 + 安装草稿推导 + 配置回填。
import { describe, expect, it } from 'vitest'
import {
  applyMcpRegistryInstallConfig,
  mcpRegistryConfigInputKey,
  searchMcpRegistry,
  withUniqueMcpServerId,
} from './mcpRegistry'

function fakeFetch(payload: unknown): typeof fetch {
  return (async () =>
    ({
      ok: true,
      status: 200,
      json: async () => payload,
    }) as Response) as unknown as typeof fetch
}

describe('mcpRegistry', () => {
  it('official npm package with required env → needs_config, stdio npx', async () => {
    const result = await searchMcpRegistry({
      source: 'official',
      fetchImpl: fakeFetch({
        servers: [
          {
            server: {
              name: 'io.example/weather',
              description: 'Weather MCP',
              version: '1.0.0',
              packages: [
                {
                  registryType: 'npm',
                  identifier: '@example/weather-mcp',
                  transport: { type: 'stdio' },
                  environmentVariables: [{ name: 'API_KEY', isRequired: true, isSecret: true }],
                },
              ],
            },
            _meta: { 'io.modelcontextprotocol.registry/official': { status: 'active' } },
          },
        ],
        metadata: {},
      }),
    })
    expect(result.items).toHaveLength(1)
    const card = result.items[0]
    expect(card.verified).toBe(true)
    const draft = card.installDraft!
    expect(draft.status).toBe('needs_config')
    expect(draft.server.transport).toBe('stdio')
    expect(draft.server.command).toBe('npx')
    expect(draft.server.args).toContain('@example/weather-mcp')

    // 回填必填 env → ready，且写入正确位置
    const input = draft.requiredConfig.find((i) => i.name === 'API_KEY')!
    const ready = applyMcpRegistryInstallConfig(draft, {
      [mcpRegistryConfigInputKey(input)]: 'secret-123',
    })
    expect(ready.status).toBe('ready')
    expect(ready.server.enabled).toBe(true)
    expect(ready.server.env.API_KEY).toBe('secret-123')
  })

  it('official ready remote → streamable_http, no config', async () => {
    const result = await searchMcpRegistry({
      source: 'official',
      fetchImpl: fakeFetch({
        servers: [
          {
            server: {
              name: 'io.example/remote',
              version: '2.0.0',
              remotes: [{ type: 'streamable-http', url: 'https://mcp.example.com/sse' }],
            },
          },
        ],
        metadata: {},
      }),
    })
    const draft = result.items[0].installDraft!
    expect(draft.status).toBe('ready')
    expect(draft.server.transport).toBe('streamable_http')
    expect(draft.server.url).toBe('https://mcp.example.com/sse')
  })

  it('withUniqueMcpServerId avoids id collisions', async () => {
    const result = await searchMcpRegistry({
      source: 'official',
      fetchImpl: fakeFetch({
        servers: [{ server: { name: 'io.example/weather', version: '1', packages: [{ registryType: 'npm', identifier: 'x', transport: { type: 'stdio' } }] } }],
        metadata: {},
      }),
    })
    const draft = result.items[0].installDraft!
    const unique = withUniqueMcpServerId(draft, [
      { id: 'weather', name: 'a', enabled: true, transport: 'stdio', url: '', command: '', args: [], env: {}, headers: {}, cwd: null, enabledTools: [] },
    ])
    expect(unique.server.id).toBe('weather-2')
  })

  it('glama env-schema card produces a manual npx draft', async () => {
    const result = await searchMcpRegistry({
      source: 'glama',
      fetchImpl: fakeFetch({
        servers: [
          {
            id: 'g1',
            name: 'example-mcp',
            description: 'x',
            npmPackage: '@example/mcp',
            environmentVariablesJsonSchema: { properties: { TOKEN: { title: 'Token' } }, required: ['TOKEN'] },
          },
        ],
        pageInfo: {},
      }),
    })
    const card = result.items[0]
    expect(card.manualDraft?.server.command).toBe('npx')
    expect(card.manualDraft?.server.args).toContain('@example/mcp')
  })
})
