import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type PluginStatus, type SkillMeta } from '../../api/tauri'
import { ComputerControlTab } from './ComputerControlTab'
import { makeChatToolsFixture } from './testFixtures'

vi.mock('../../api/tauri', async importOriginal => ({
  ...await importOriginal<typeof import('../../api/tauri')>(),
  api: {
    computerControlCheck: vi.fn(), computerControlStatus: vi.fn(), computerControlInstall: vi.fn(), computerControlUpdate: vi.fn(), chatSkillsList: vi.fn(),
    pluginsList: vi.fn(), pluginsRunOfficialInstall: vi.fn(), pluginsSetEnabled: vi.fn(), openExternal: vi.fn(),
  },
}))

const playwrightSkill = { id: 'playwright-cli', name: 'playwright-cli', source: 'user', description: 'Browser automation', recommendedTools: [] } satisfies SkillMeta
const cuaSkill = { id: 'cua-driver', name: 'cua-driver', source: 'user', description: 'Desktop automation', recommendedTools: [] } satisfies SkillMeta
const cuaMcp = {
  id: 'computer-control-cua-driver', name: 'Cua Driver', enabled: true, transport: 'stdio', url: '',
  command: 'cua-driver', args: ['mcp'], env: {}, headers: {}, cwd: null, enabledTools: [],
}
const pluginStatus = (overrides: Partial<PluginStatus>): PluginStatus => ({
  id: 'ego-lite', name: 'ego lite', description: '', binary: 'ego-browser', tags: [], homepage: '', repo: '',
  installed: true, enabled: true, version: '0.4.0', path: null, source: 'system', hasSkill: true,
  hasMcp: false, skillIds: ['ego-browser'], skillCount: 1, mcpCount: 0, skillActive: true,
  mcpActive: false, mcpServerId: null, canInstall: true, ...overrides,
})

describe('ComputerControlTab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.sessionStorage.clear()
    vi.mocked(api.computerControlStatus).mockImplementation(async tool => {
      if (tool === 'cua') return { currentVersion: '0.28.1', latestVersion: '0.28.1', updateAvailable: false }
      throw new Error('playwright-cli not found')
    })
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [] })
    vi.mocked(api.pluginsList).mockResolvedValue([])
  })

  it('shows only a compact ready state or install action', async () => {
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [cuaSkill] })
    const onChange = vi.fn()
    const tools = makeChatToolsFixture()
    tools.enabled = true
    tools.servers = [cuaMcp]
    render(<ComputerControlTab lang="zh" tools={tools} onChange={onChange} />)
    expect(await screen.findByText('v0.28.1 · 1 Skill · 1 MCP')).toBeTruthy()
    expect(screen.getAllByText('未安装').length).toBeGreaterThan(0)
    expect(screen.getByRole('switch', { name: 'Cua Driver 控制' })).toBeTruthy()
    expect(screen.getAllByRole('button', { name: '安装' }).length).toBeGreaterThan(0)
    expect(screen.getByText('电脑操作')).toBeTruthy()
    expect(screen.getByText('浏览器操作')).toBeTruthy()
    expect(screen.queryByText('0 Skill · 0 MCP')).toBeNull()
    expect(screen.queryByText(/已连接|可操作/)).toBeNull()
    expect(screen.queryByText(/CLI 已安装|检测详情/)).toBeNull()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('installs and enables the official Skill using current settings', async () => {
    let finish!: (value: SkillMeta) => void
    vi.mocked(api.computerControlInstall).mockReturnValue(new Promise(resolve => { finish = resolve }))
    const onChange = vi.fn()
    const tools = { ...makeChatToolsFixture(), disabledSkillIds: ['playwright-cli', 'other'] }
    const { rerender } = render(<ComputerControlTab lang="zh" tools={tools} onChange={onChange} />)
    await screen.findAllByText('未安装')
    fireEvent.click(screen.getAllByRole('button', { name: '安装' })[1])
    expect(api.computerControlInstall).toHaveBeenCalledWith('playwright')
    const latest = { ...tools, disabledSkillIds: [...tools.disabledSkillIds, 'newly-disabled'] }
    rerender(<ComputerControlTab lang="zh" tools={latest} onChange={onChange} />)
    finish(playwrightSkill)
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      enabled: true, disabledSkillIds: ['other', 'newly-disabled'],
      nativeTools: expect.objectContaining({ skillRuntime: true, runCommand: true, readFile: true }),
    })))
  })

  it('surfaces install failure and keeps settings unchanged', async () => {
    vi.mocked(api.computerControlInstall).mockRejectedValue(new Error('npm unavailable'))
    const onChange = vi.fn()
    render(<ComputerControlTab lang="zh" tools={makeChatToolsFixture()} onChange={onChange} />)
    await screen.findAllByText('未安装')
    fireEvent.click(screen.getAllByRole('button', { name: '安装' })[1])
    expect(await screen.findByRole('alert')).toHaveTextContent('安装失败，请稍后重试。')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('uses the existing disabledSkillIds switch', async () => {
    vi.mocked(api.computerControlStatus).mockResolvedValue({ currentVersion: '1.0.0', latestVersion: '1.0.0', updateAvailable: false })
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [playwrightSkill] })
    const tools = makeChatToolsFixture()
    tools.enabled = true
    const onChange = vi.fn()
    render(<ComputerControlTab lang="en" tools={tools} onChange={onChange} />)
    await screen.findByText('v1.0.0 · 1 Skill')
    fireEvent.click(screen.getByRole('switch', { name: 'Playwright CLI control' }))
    expect(onChange).toHaveBeenCalledWith({ disabledSkillIds: ['playwright-cli'] })
  })

  it('controls the Cua Skill and MCP together', async () => {
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [cuaSkill] })
    const tools = makeChatToolsFixture()
    tools.enabled = true
    tools.servers = [{ ...cuaMcp, id: 'plugin-cua-driver', connectorId: 'plugin:cua-driver' }]
    const onChange = vi.fn()
    render(<ComputerControlTab lang="zh" tools={tools} onChange={onChange} />)
    await screen.findByText('v0.28.1 · 1 Skill · 1 MCP')
    fireEvent.click(screen.getByRole('switch', { name: 'Cua Driver 控制' }))
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      disabledSkillIds: ['cua-driver'],
      servers: [expect.objectContaining({
        id: 'computer-control-cua-driver',
        enabled: false,
        args: ['mcp'],
      })],
    }))
  })

  it('groups ego lite with browsers and OfficeCLI with documents', async () => {
    vi.mocked(api.pluginsList).mockResolvedValue([
      pluginStatus({ id: 'ego-lite', name: 'ego lite', version: '0.4.0' }),
      pluginStatus({
        id: 'officecli', name: 'OfficeCLI', binary: 'officecli', version: '1.8.2',
        hasMcp: true, skillIds: Array.from({ length: 12 }, (_, index) => `office-${index}`),
        skillCount: 12, mcpCount: 1, mcpActive: true, mcpServerId: 'plugin-officecli',
      }),
    ])
    render(<ComputerControlTab lang="zh" tools={makeChatToolsFixture()} onChange={vi.fn()} />)
    expect(await screen.findByText('v0.4.0 · 1 Skill')).toBeTruthy()
    expect(screen.getByText('文档操作')).toBeTruthy()
    expect(screen.getByText('v1.8.2 · 12 Skill · 1 MCP')).toBeTruthy()
    expect(screen.getByRole('switch', { name: 'ego lite 控制' })).toBeTruthy()
    expect(screen.getByRole('switch', { name: 'OfficeCLI 控制' })).toBeTruthy()
  })

  it('updates an installed migrated tool through its switch', async () => {
    const ego = pluginStatus({ id: 'ego-lite', enabled: true })
    vi.mocked(api.pluginsList).mockResolvedValue([ego])
    vi.mocked(api.pluginsSetEnabled).mockResolvedValue({
      ok: true,
      message: '',
      status: { ...ego, enabled: false },
    })
    render(<ComputerControlTab lang="zh" tools={makeChatToolsFixture()} onChange={vi.fn()} />)
    const toggle = await screen.findByRole('switch', { name: 'ego lite 控制' })
    vi.mocked(api.computerControlStatus).mockClear()
    vi.mocked(api.chatSkillsList).mockClear()
    vi.mocked(api.pluginsList).mockClear()
    fireEvent.click(toggle)
    await waitFor(() => expect(api.pluginsSetEnabled).toHaveBeenCalledWith('ego-lite', false))
    expect(api.computerControlStatus).not.toHaveBeenCalled()
    expect(api.chatSkillsList).not.toHaveBeenCalled()
    expect(api.pluginsList).not.toHaveBeenCalled()
  })

  it('reuses the completed detection result when the page is opened again', async () => {
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [cuaSkill] })
    const tools = makeChatToolsFixture()
    tools.enabled = true
    tools.servers = [cuaMcp]

    const first = render(<ComputerControlTab lang="zh" tools={tools} onChange={vi.fn()} />)
    await screen.findByText('v0.28.1 · 1 Skill · 1 MCP')
    first.unmount()
    vi.mocked(api.computerControlStatus).mockClear()
    vi.mocked(api.chatSkillsList).mockClear()
    vi.mocked(api.pluginsList).mockClear()

    render(<ComputerControlTab lang="zh" tools={tools} onChange={vi.fn()} />)
    expect(screen.queryByText('正在检测…')).toBeNull()
    expect(screen.getByText('v0.28.1 · 1 Skill · 1 MCP')).toBeTruthy()
    expect(api.computerControlStatus).not.toHaveBeenCalled()
    expect(api.chatSkillsList).not.toHaveBeenCalled()
    expect(api.pluginsList).not.toHaveBeenCalled()
  })

  it('updates the Cua binary, MCP runtime, and official skill when a release is available', async () => {
    vi.mocked(api.computerControlStatus).mockImplementation(async tool => {
      if (tool === 'cua') return { currentVersion: '0.28.1', latestVersion: '0.28.2', updateAvailable: true }
      throw new Error('playwright-cli not found')
    })
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [cuaSkill] })
    vi.mocked(api.computerControlUpdate).mockResolvedValue(cuaSkill)
    const tools = makeChatToolsFixture()
    tools.enabled = true
    tools.servers = [cuaMcp]

    render(<ComputerControlTab lang="zh" tools={tools} onChange={vi.fn()} />)
    const button = await screen.findByRole('button', { name: '更新' })
    expect(button).toHaveAttribute('title', '最新版本 v0.28.2')
    fireEvent.click(button)
    await waitFor(() => expect(api.computerControlUpdate).toHaveBeenCalledWith('cua'))
  })

  it('shows the backend error when a native tool update fails', async () => {
    vi.mocked(api.computerControlStatus).mockImplementation(async tool => {
      if (tool === 'cua') return { currentVersion: '0.28.1', latestVersion: '0.28.2', updateAvailable: true }
      throw new Error('playwright-cli not found')
    })
    vi.mocked(api.chatSkillsList).mockResolvedValue({ success: true, skills: [cuaSkill] })
    vi.mocked(api.computerControlUpdate).mockRejectedValue(new Error('installer exited with code 1'))
    const tools = makeChatToolsFixture()
    tools.enabled = true
    tools.servers = [cuaMcp]

    render(<ComputerControlTab lang="zh" tools={tools} onChange={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '更新' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('更新失败：installer exited with code 1')
  })
})
