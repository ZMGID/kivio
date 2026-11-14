import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { ExternalAgentsSettings } from './ExternalAgentsSettings'
import { externalCliSettingsApi, onExternalCliInstallLog } from '../api/externalCliSettings'
import { resetCliInstallJobsForTests } from './cliInstallJobs'
import { resetCliInstallInfoForTests } from './externalAgents/cliMaintenance'
import type { Settings as SettingsData } from '../api/tauri'
import type { ProviderCapabilities } from '../generated/chatProtocol'

const preparedInstall = vi.hoisted(() => ({
  planId: 'prepared-plan',
  manualReason: null,
  binaryPath: '/prefix/bin/cli',
  targetVersion: '1.1.0',
  environment: 'host',
  source: 'npm',
}))

vi.mock('../api/externalCliSettings', () => ({
  externalCliSettingsApi: {
    detectExternalAgents: vi.fn(),
    detectExternalAgentModels: vi.fn().mockResolvedValue({ models: [], reasoningOptions: [] }),
    externalCliInstallInfo: vi.fn().mockResolvedValue({
      agentId: 'claude',
      localVersion: '1.0.0',
      latestVersion: '1.1.0',
      updateAvailable: true,
      command: 'npm install -g @anthropic-ai/claude-code@latest',
      docsUrl: 'https://docs.claude.com',
      ...preparedInstall,
      configDir: '/home/u/.claude',
    }),
    externalCliInstall: vi.fn(),
    externalCliImportCcSwitch: vi.fn(),
    externalCliScanCcSwitch: vi.fn().mockResolvedValue({ providers: [], skipped: 0 }),
    externalCliFetchRelayModels: vi.fn().mockResolvedValue([]),
    dshPluginSettingsGet: vi.fn().mockResolvedValue(null),
    dshPluginSettingsSave: vi.fn(),
    dshPluginInventory: vi.fn().mockResolvedValue([]),
    dshOpenSettingsFile: vi.fn(),
    dshOfficialCredentialStatus: vi.fn().mockResolvedValue({ configured: false, writable: true }),
    dshOfficialCredentialSave: vi.fn(),
    dshNativeProviderGet: vi.fn(),
    dshNativeProviderDelete: vi.fn(),
    piExtensionsInventory: vi.fn().mockResolvedValue({
      agentDir: '/home/u/.pi/agent',
      extensionsDir: '/home/u/.pi/agent/extensions',
      packages: [],
      localExtensions: [],
    }),
    piExtensionSetEnabled: vi.fn(),
    piExtensionInstall: vi.fn(),
    piExtensionUpdate: vi.fn(),
    piExtensionRemove: vi.fn(),
    piExtensionOpen: vi.fn(),
    piExtensionsOpenDir: vi.fn(),
    piSkillsInventory: vi.fn().mockResolvedValue({
      agentDir: '/home/u/.pi/agent',
      piSkillsDir: '/home/u/.pi/agent/skills',
      agentsSkillsDir: '/home/u/.agents/skills',
      skillCommandsEnabled: true,
      configuredPaths: [],
      skills: [],
    }),
    piSkillSetEnabled: vi.fn(),
    piSkillCommandsSetEnabled: vi.fn(),
    piSkillAddPath: vi.fn(),
    piSkillRemovePath: vi.fn(),
    piSkillRemove: vi.fn(),
    piSkillOpen: vi.fn(),
    piSkillsOpenDir: vi.fn(),
  },
  onExternalCliInstallLog: vi.fn().mockResolvedValue(() => {}),
  onExternalAgentsUpdated: vi.fn().mockResolvedValue(() => {}),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

const mockDetect = vi.mocked(externalCliSettingsApi.detectExternalAgents)
const mockInstallInfo = vi.mocked(externalCliSettingsApi.externalCliInstallInfo)
const mockInstall = vi.mocked(externalCliSettingsApi.externalCliInstall)
const mockOnInstallLog = vi.mocked(onExternalCliInstallLog)
const mockOfficialKeyStatus = vi.mocked(externalCliSettingsApi.dshOfficialCredentialStatus)
const mockOfficialKeySave = vi.mocked(externalCliSettingsApi.dshOfficialCredentialSave)
const mockNativeGet = vi.mocked(externalCliSettingsApi.dshNativeProviderGet)
const mockNativeDelete = vi.mocked(externalCliSettingsApi.dshNativeProviderDelete)

const capabilities: ProviderCapabilities = {
  steering: 'unavailable',
  followUp: 'fallback',
  imageInput: 'native',
  rewind: 'fallback',
  conversationBranch: 'fallback',
  importHistory: 'unavailable',
  account: 'unavailable',
}

function renderPanel(
  chat: Partial<NonNullable<SettingsData['chat']>> = {},
  updateChat = vi.fn(),
) {
  return {
    updateChat,
    ...render(
      <ExternalAgentsSettings
        lang="zh"
        settings={{ chat } as SettingsData}
        updateChat={updateChat}
      />,
    ),
  }
}

describe('ExternalAgentsSettings', () => {
  beforeEach(() => {
    resetCliInstallJobsForTests()
    resetCliInstallInfoForTests()
    mockDetect.mockReset()
    mockInstallInfo.mockReset()
    mockInstall.mockReset()
    mockOnInstallLog.mockReset()
    mockOfficialKeyStatus.mockReset()
    mockOfficialKeySave.mockReset()
    mockNativeGet.mockReset()
    mockNativeDelete.mockReset()
    mockOfficialKeyStatus.mockResolvedValue({ configured: false, writable: true })
    mockOfficialKeySave.mockResolvedValue({ configured: true, writable: true })
    mockNativeDelete.mockResolvedValue(undefined)
    mockDetect.mockResolvedValue([
      {
        id: 'claude',
        name: 'Claude Code',
        available: true,
        capabilities,
        path: '/usr/local/bin/claude',
        version: '1.0.0',
        models: [{ id: 'default', label: 'Default' }],
        authStatus: 'ok',
      },
      {
        id: 'codex',
        name: 'Codex',
        available: false,
        capabilities,
        models: [],
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'claude',
      localVersion: '1.0.0',
      latestVersion: '1.1.0',
      updateAvailable: true,
      command: 'npm install -g @anthropic-ai/claude-code@latest',
      docsUrl: 'https://docs.claude.com',
      ...preparedInstall,
      configDir: '/home/u/.claude',
    })
    mockInstall.mockResolvedValue()
    mockOnInstallLog.mockResolvedValue(() => {})
  })

  it('lists every CLI with its state and selects the first available one', async () => {
    renderPanel()

    await waitFor(() => {
      expect(screen.getAllByText('Claude Code').length).toBeGreaterThan(0)
    })

    // 左栏第二行：装了的说登录态，没装的说未安装。没装的没有启用开关。
    expect(screen.getAllByText('已登录').length).toBeGreaterThan(0)
    expect(screen.getByText('未安装')).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Claude Code 启用' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.queryByRole('switch', { name: 'Codex 启用' })).not.toBeInTheDocument()
    // 首个可用的进详情面板：运行环境这一区只在选中项上渲染。
    expect(screen.getByText('可执行文件')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('/usr/local/bin/claude')).toHaveValue('')
    expect(screen.getByText('环境变量')).toBeInTheDocument()
    expect(await screen.findByText('/home/u/.claude')).toBeInTheDocument()
    expect(screen.queryByText('已检测到')).not.toBeInTheDocument()
    expect(mockDetect).toHaveBeenCalled()
  })

  it('flags a CLI that still needs a sign-in', async () => {
    mockDetect.mockResolvedValue([
      { id: 'claude', name: 'Claude Code', available: true, capabilities, models: [], authStatus: 'auth_required' },
    ])
    renderPanel()

    expect(await screen.findByText('未登录')).toBeInTheDocument()
    expect(screen.getByText('未登录。在终端运行该 CLI 完成登录，或在下方添加一个供应商。')).toBeInTheDocument()
  })

  it('commits a typed binary path only when the field loses focus', async () => {
    const { updateChat } = renderPanel()
    const input = await screen.findByPlaceholderText('/usr/local/bin/claude')

    fireEvent.change(input, { target: { value: '/opt/claude' } })
    expect(updateChat).not.toHaveBeenCalled()
    fireEvent.blur(input)
    expect(updateChat).toHaveBeenCalledWith({ externalCliAgents: { claude: { path: '/opt/claude' } } })
  })

  it('adds a custom model from the inline form and lists it once', async () => {
    vi.mocked(externalCliSettingsApi.detectExternalAgentModels).mockResolvedValueOnce({
      // 后端的探测结果里已经并入了保存过的自定义模型。
      models: [{ id: 'sonnet', label: 'Sonnet' }, { id: 'mine', label: 'Mine' }],
      reasoningOptions: [],
      reasoningByModel: {},
      source: 'probed',
    })
    const { updateChat } = renderPanel({
      externalCliAgents: { claude: { customModels: [{ id: 'mine', label: 'Mine' }] } },
    })

    expect(await screen.findByText('sonnet')).toBeInTheDocument()
    expect(screen.getAllByText('mine')).toHaveLength(1)
    expect(screen.getByText('探测到 1 个 · 自定义 1 个')).toBeInTheDocument()

    const add = screen.getByRole('button', { name: '添加模型' })
    expect(add).toBeDisabled()
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'sonnet' } })
    expect(add).toBeDisabled()
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: ' opus-next ' } })
    fireEvent.click(add)
    expect(updateChat).toHaveBeenCalledWith({
      externalCliAgents: {
        claude: {
          customModels: [
            { id: 'mine', label: 'Mine' },
            { id: 'opus-next', label: '' },
          ],
        },
      },
    })
  })

  it('removes an adopted provider through the settings draft without deleting native files early', async () => {
    mockDetect.mockResolvedValue([{
      id: 'dsh', name: 'DeepSeek Harness', available: true, capabilities, models: [],
      nativeProviders: [{ id: 'relay', name: 'Relay', modelCount: 1, isDefault: true }],
    }])
    const { updateChat } = renderPanel({ externalCliAgents: { dsh: {
      currentProvider: 'managed',
      providers: [{ id: 'managed', name: 'Relay', nativeProviderId: 'relay', env: [], configToml: '', authJson: '' }],
    } } })
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    fireEvent.click(await screen.findByRole('button', { name: '删除' }))
    await waitFor(() => expect(updateChat).toHaveBeenCalledWith(expect.objectContaining({
      externalCliAgents: { dsh: { currentProvider: '', providers: [] } },
    })))
    expect(mockNativeDelete).not.toHaveBeenCalled()
  })

  it('shows an explicit update status and update action only when a newer version exists', async () => {
    renderPanel()

    await waitFor(() => {
      expect(screen.getByText('可更新到 1.1.0')).toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: '更新' })).toBeInTheDocument()
    expect(screen.getByText('npm install -g @anthropic-ai/claude-code@latest')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '复制命令' })).toBeInTheDocument()
  })

  it('rescans all agents after an update command finishes', async () => {
    renderPanel()

    const update = await screen.findByRole('button', { name: '更新' })
    fireEvent.click(update)

    await waitFor(() => {
      expect(mockInstall).toHaveBeenCalledWith('claude', 'prepared-plan')
      expect(mockDetect).toHaveBeenCalledWith(true)
    })
    expect(screen.getByRole('status')).toHaveTextContent('安装完成')
    expect(screen.getByRole('status')).not.toHaveTextContent('DeepSeek')
    expect(screen.queryByText('安装日志')).not.toBeInTheDocument()
  })

  it('keeps an in-flight update visible after switching CLIs and coming back', async () => {
    let finishInstall: () => void = () => {}
    mockInstall.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finishInstall = resolve
        }),
    )
    let emit:
      | ((event: {
          agentId: string
          planId: string
          line: string | null
          done: boolean
          success: boolean
        }) => void)
      | undefined
    mockOnInstallLog.mockImplementation(async (handler) => {
      emit = handler
      return () => {}
    })

    renderPanel()
    fireEvent.click(await screen.findByRole('button', { name: '更新' }))
    await waitFor(() => {
      expect(screen.getByRole('status')).toHaveTextContent('执行中')
    })
    expect(screen.getByLabelText('执行中…')).toBeInTheDocument()

    fireEvent.click(screen.getByText('Codex'))
    await waitFor(() => {
      expect(screen.queryByRole('status')).not.toBeInTheDocument()
    })
    expect(screen.getByLabelText('执行中…')).toBeInTheDocument()

    emit?.({
      agentId: 'claude',
      planId: 'prepared-plan',
      line: '$ npm install -g @anthropic-ai/claude-code@latest',
      done: false,
      success: false,
    })

    fireEvent.click(screen.getByText('Claude Code'))
    await waitFor(() => {
      expect(screen.getByRole('status')).toHaveTextContent('执行中')
    })
    expect(
      screen.getByText('$ npm install -g @anthropic-ai/claude-code@latest'),
    ).toBeInTheDocument()

    finishInstall()
    await waitFor(() => {
      expect(screen.getByRole('status')).toHaveTextContent('安装完成')
    })
    expect(screen.getByRole('status')).not.toHaveTextContent('DeepSeek')
  })

  it('reminds about the DeepSeek key only after a dsh install finishes', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.7',
      updateAvailable: true,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/deepseek-harness',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel()
    fireEvent.click(await screen.findByRole('button', { name: '更新' }))
    await waitFor(() => {
      expect(screen.getByRole('status')).toHaveTextContent('DeepSeek')
    })
  })

  it('shows the prerelease suffix and offers an update from rc.6 to rc.7', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.7',
      updateAvailable: true,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/deepseek-harness',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel()

    await waitFor(() => {
      expect(screen.getAllByText('0.1.0-rc.6').length).toBeGreaterThan(0)
    })
    expect(screen.getByText('可更新到 0.1.0-rc.7')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '更新' })).toBeInTheDocument()
    expect(screen.queryByText('已是最新')).not.toBeInTheDocument()
  })

  it('keeps a matching prerelease as up to date', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.7',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.7',
      latestVersion: '0.1.0-rc.7',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/deepseek-harness',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel()

    await waitFor(() => {
      expect(screen.getAllByText('0.1.0-rc.7').length).toBeGreaterThan(0)
    })
    expect(screen.getByText('已是最新')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '更新' })).not.toBeInTheDocument()
  })

  it('shows up-to-date status without an update action', async () => {
    mockInstallInfo.mockResolvedValue({
      agentId: 'claude',
      localVersion: '1.0.0',
      latestVersion: '1.0.0',
      updateAvailable: false,
      command: 'npm install -g @anthropic-ai/claude-code@latest',
      docsUrl: 'https://docs.claude.com',
      ...preparedInstall,
      configDir: '/home/u/.claude',
    })

    renderPanel()

    await waitFor(() => {
      expect(screen.getByText('已是最新')).toBeInTheDocument()
    })
    expect(screen.queryByRole('button', { name: '更新' })).not.toBeInTheDocument()
  })

  it('offers a repair update when the CLI is on PATH but version cannot be read', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: null,
      latestVersion: '0.1.0-rc.6',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/deepseek-harness',
      ...preparedInstall,
      configDir: null,
    })

    renderPanel()
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '更新' })).toBeInTheDocument()
    })
    expect(screen.getByText('已写入但无法启动')).toBeInTheDocument()
    expect(screen.queryByText('未安装')).not.toBeInTheDocument()
  })

  it('does not offer an update when the latest version cannot be checked', async () => {
    mockInstallInfo.mockResolvedValue({
      agentId: 'claude',
      localVersion: '1.0.0',
      latestVersion: null,
      updateAvailable: false,
      command: 'npm install -g @anthropic-ai/claude-code@latest',
      docsUrl: 'https://docs.claude.com',
      ...preparedInstall,
      configDir: '/home/u/.claude',
    })

    renderPanel()

    await waitFor(() => {
      expect(screen.getByText('无法确认最新版本')).toBeInTheDocument()
    })
    expect(screen.queryByRole('button', { name: '更新' })).not.toBeInTheDocument()
  })

  it('explains when an update exists but the install source cannot be updated safely', async () => {
    mockInstallInfo.mockResolvedValue({
      agentId: 'gemini',
      localVersion: '1.0.0',
      latestVersion: '1.1.0',
      updateAvailable: true,
      command: null,
      docsUrl: 'https://www.geminicli.com/docs/get-started/installation',
      ...preparedInstall,
      configDir: '/home/u/.gemini',
      planId: null,
      manualReason: '无法确认此安装的 npm 来源，请按官方文档维护',
    })

    renderPanel()

    await waitFor(() => {
      expect(screen.getByText('可更新到 1.1.0')).toBeInTheDocument()
    })
    expect(screen.getByText('无法确认此安装的 npm 来源，请按官方文档维护')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '更新' })).not.toBeInTheDocument()
    // 不能自动更新时，动作位换成通往文档的链接。
    expect(screen.getAllByRole('link', { name: '官方文档' })).toHaveLength(2)
  })

  it('keeps the manual-update reason quiet while the CLI is already current', async () => {
    mockInstallInfo.mockResolvedValue({
      agentId: 'claude',
      localVersion: '1.1.0',
      latestVersion: '1.1.0',
      updateAvailable: false,
      command: null,
      docsUrl: 'https://docs.claude.com',
      ...preparedInstall,
      configDir: null,
      planId: null,
      manualReason: '无法验证此 CLI 的 npm 安装归属，请按官方文档更新',
    })

    renderPanel()

    expect(await screen.findByText('已是最新')).toBeInTheDocument()
    expect(screen.queryByText('无法验证此 CLI 的 npm 安装归属，请按官方文档更新')).not.toBeInTheDocument()
    expect(screen.queryByText('配置目录')).not.toBeInTheDocument()
  })

  it('edits the environment variables injected into the CLI', async () => {
    const { updateChat } = renderPanel({
      externalCliAgents: { claude: { env: [{ key: 'HTTPS_PROXY', value: 'http://127.0.0.1:7890' }] } },
    })

    expect(await screen.findByDisplayValue('HTTPS_PROXY')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '添加变量' }))
    expect(updateChat).toHaveBeenCalledWith({
      externalCliAgents: {
        claude: {
          env: [
            { key: 'HTTPS_PROXY', value: 'http://127.0.0.1:7890' },
            { key: '', value: '' },
          ],
        },
      },
    })
  })

  it('masks the value of a variable that looks like a secret', async () => {
    renderPanel({
      externalCliAgents: { claude: { env: [{ key: 'ANTHROPIC_API_KEY', value: 'sk-secret' }] } },
    })

    expect(await screen.findByDisplayValue('sk-secret')).toHaveAttribute('type', 'password')
  })

  it('keeps the previous list when a rescan fails', async () => {
    renderPanel()
    await screen.findByText('可执行文件')

    mockDetect.mockRejectedValueOnce(new Error('boom'))
    vi.spyOn(console, 'error').mockImplementation(() => {})
    fireEvent.click(screen.getByRole('button', { name: '重新扫描' }))

    await waitFor(() => expect(mockDetect).toHaveBeenCalledWith(true))
    expect(screen.getAllByText('Claude Code').length).toBeGreaterThan(0)
    expect(screen.getByText('Codex')).toBeInTheDocument()
  })

  it('shows only install and path for a CLI that is not installed', async () => {
    renderPanel()
    await waitFor(() => {
      expect(screen.getByText('供应商')).toBeInTheDocument()
    })

    fireEvent.click(screen.getByText('Codex'))
    await waitFor(() => {
      expect(screen.queryByText('供应商')).not.toBeInTheDocument()
    })
    expect(screen.queryByText('模型')).not.toBeInTheDocument()
    expect(screen.getByText('可执行文件')).toBeInTheDocument()
  })

  it('toggles a CLI from its list row without moving it', async () => {
    const { updateChat } = renderPanel({ externalCliAgents: { claude: { disabled: true } } })

    expect(await screen.findByText('已停用')).toBeInTheDocument()
    const toggle = screen.getByRole('switch', { name: 'Claude Code 启用' })
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    // 停用的仍排在没装的前面。
    const names = [...document.querySelectorAll('.kv-cli-nav-name')].map((node) => node.textContent)
    expect(names).toEqual(['Claude Code', 'Codex'])

    fireEvent.click(toggle)
    expect(updateChat).toHaveBeenCalledWith({ externalCliAgents: { claude: { disabled: false } } })
  })

  it('activating a provider writes currentProvider', async () => {
    const { updateChat } = renderPanel({
      externalCliAgents: {
        claude: {
          providers: [
            { id: 'relay-1', name: 'Loki', env: [{ key: 'ANTHROPIC_BASE_URL', value: 'https://relay' }] },
          ],
        },
      },
    })

    await waitFor(() => {
      expect(screen.getByText('Loki')).toBeInTheDocument()
    })
    // 单选型 CLI 用行首的单选圈切换当前供应商。
    expect(screen.getByRole('radio', { name: '使用 CLI 自身配置' })).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(screen.getByRole('radio', { name: 'Loki' }))
    expect(updateChat).toHaveBeenCalledWith(
      expect.objectContaining({
        externalCliAgents: expect.objectContaining({
          claude: expect.objectContaining({ currentProvider: 'relay-1' }),
        }),
      }),
    )
  })

  it('keeps Pi providers independent while allowing one default', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'pi',
        name: 'Pi',
        available: true,
        capabilities,
        path: '/usr/local/bin/pi',
        version: '0.50.2',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'pi',
      localVersion: '0.50.2',
      latestVersion: '0.50.2',
      updateAvailable: false,
      command: 'npm install -g @mariozechner/pi-coding-agent@latest',
      docsUrl: 'https://github.com/badlogic/pi-mono',
      ...preparedInstall,
      configDir: '/home/u/.pi/agent',
    })

    const { updateChat } = renderPanel({
      externalCliAgents: {
        pi: {
          currentProvider: 'relay-1',
          providers: [
            { id: 'relay-1', name: 'Relay One' },
            { id: 'relay-2', name: 'Relay Two' },
          ],
        },
      },
    })

    // 左栏第二行也写着当前默认供应商的名字，这里要的是供应商列表里的那一行。
    const first = (await screen.findAllByText('Relay One'))
      .map((element) => element.closest<HTMLElement>('.kv-row'))
      .find((row) => row !== null)!
    const second = screen.getByText('Relay Two').closest<HTMLElement>('.kv-row')!
    expect(within(first).getByText('默认')).toBeInTheDocument()
    // 已启用但不是默认的那条：开关本身就是状态，不再另挂徽章。
    expect(within(second).queryByText('默认')).not.toBeInTheDocument()
    expect(within(second).getByRole('switch')).toHaveAttribute('aria-checked', 'true')
    expect(within(first).getByRole('switch')).toHaveAttribute('aria-checked', 'true')

    fireEvent.click(within(first).getByRole('switch'))
    expect(updateChat).toHaveBeenCalledWith(
      expect.objectContaining({
        externalCliAgents: expect.objectContaining({
          pi: expect.objectContaining({
            currentProvider: '',
            providers: expect.arrayContaining([
              expect.objectContaining({ id: 'relay-1', disabled: true }),
            ]),
          }),
        }),
      }),
    )
    updateChat.mockClear()
    fireEvent.click(within(second).getByRole('button', { name: '设为默认' }))
    expect(updateChat).toHaveBeenCalledWith(
      expect.objectContaining({
        externalCliAgents: expect.objectContaining({
          pi: expect.objectContaining({ currentProvider: 'relay-2' }),
        }),
      }),
    )
  })

  it('shows the empty state when no provider is configured', async () => {
    renderPanel()
    await waitFor(() => {
      expect(screen.getByText('供应商')).toBeInTheDocument()
    })
    expect(screen.getByText('当前沿用 CLI 自己的配置与登录态。添加供应商后，可让 Kivio 拉起的进程改走第三方中转。')).toBeInTheDocument()
  })

  it('lists official DeepSeek as the in-use dsh provider', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.6',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/dsh',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel()
    await waitFor(() => {
      expect(screen.getByText('DeepSeek')).toBeInTheDocument()
    })
    expect(screen.getByText('默认')).toBeInTheDocument()
    expect(screen.queryByText('使用 CLI 自身配置')).not.toBeInTheDocument()
    expect(screen.queryByText('当前沿用 CLI 自己的配置与登录态。添加供应商后，可让 Kivio 拉起的进程改走第三方中转。')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '编辑供应商' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '删除' })).not.toBeInTheDocument()
  })

  it('lets the user edit or delete a native dsh relay', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
        nativeProviders: [
          { id: 'deepseek-official', name: 'DeepSeek', modelCount: 2, isDefault: false },
          {
            id: 'gpt',
            name: 'gpt',
            baseUrl: 'https://api.example/v1',
            modelCount: 4,
            isDefault: true,
          },
        ],
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.6',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/dsh',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })
    mockNativeGet.mockResolvedValue({
      id: 'gpt',
      name: 'gpt',
      baseUrl: 'https://api.example/v1',
      api: 'openai-responses',
      apiKey: 'sk-native',
      apiKeyEnv: 'GPT_API_KEY',
      models: [{ id: 'gpt-5.6-sol', name: 'gpt-5.6-sol' }],
      defaultModel: 'gpt-5.6-sol',
    })

    const { updateChat } = renderPanel()
    await waitFor(() => {
      expect(screen.getByText('gpt')).toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: '编辑供应商' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '删除' })).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '编辑供应商' }))
    await waitFor(() => {
      expect(mockNativeGet).toHaveBeenCalledWith('gpt')
    })
    expect(await screen.findByText('编辑 DeepSeek Harness 供应商')).toBeInTheDocument()
    fireEvent.click(screen.getByText('取消'))
    await waitFor(() => {
      expect(screen.queryByText('编辑 DeepSeek Harness 供应商')).not.toBeInTheDocument()
    })

    vi.spyOn(window, 'confirm').mockReturnValue(true)
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => {
      expect(mockNativeDelete).toHaveBeenCalledWith('gpt')
    })
    expect(updateChat).not.toHaveBeenCalled()
  })

  it('hides a native dsh relay after it is adopted into Kivio providers', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
        nativeProviders: [
          { id: 'deepseek-official', name: 'DeepSeek', modelCount: 2, isDefault: false },
          {
            id: 'gpt',
            name: 'gpt',
            baseUrl: 'https://api.example/v1',
            modelCount: 4,
            isDefault: true,
          },
        ],
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.6',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/dsh',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel({
      externalCliAgents: {
        dsh: {
          providers: [
            {
              id: 'p-dsh-gpt',
              name: 'gpt',
              nativeProviderId: 'gpt',
              env: [{ key: 'KIVIO_DSH_GPT_API_KEY', value: 'sk' }],
            },
          ],
        },
      },
    })
    await waitFor(() => {
      expect(screen.getByText('gpt')).toBeInTheDocument()
    })
    expect(screen.getAllByText('gpt')).toHaveLength(1)
    expect(screen.getByRole('button', { name: '编辑供应商' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '删除' })).toBeInTheDocument()
    expect(screen.queryByText('https://api.example/v1')).not.toBeInTheDocument()
  })

  it('opens dsh plugins on a secondary page', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.6',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/dsh',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel()
    await waitFor(() => {
      expect(screen.getByText('供应商')).toBeInTheDocument()
    })
    expect(screen.queryByRole('tab', { name: '插件配置' })).not.toBeInTheDocument()
    const plugins = screen.getByText('插件')

    fireEvent.click(plugins.closest('button')!)
    expect(screen.getByRole('tab', { name: '插件配置' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /返回/ })).toBeInTheDocument()
    expect(screen.queryByText('供应商')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /返回/ }))
    expect(screen.queryByRole('tab', { name: '插件配置' })).not.toBeInTheDocument()
    expect(screen.getByText('供应商')).toBeInTheDocument()
  })

  it('opens Pi extension management as a full secondary page', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'pi',
        name: 'Pi',
        available: true,
        capabilities,
        path: '/usr/local/bin/pi',
        version: '0.50.2',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'pi',
      localVersion: '0.50.2',
      latestVersion: '0.50.2',
      updateAvailable: false,
      command: 'npm install -g @earendil-works/pi-coding-agent@latest',
      docsUrl: 'https://github.com/badlogic/pi-mono',
      ...preparedInstall,
      configDir: '/home/u/.pi/agent',
    })

    renderPanel()
    const entry = await screen.findByText('扩展管理')
    fireEvent.click(entry.closest('button')!)

    expect(await screen.findByText('Pi Package')).toBeInTheDocument()
    expect(screen.queryByPlaceholderText('搜索 CLI')).not.toBeInTheDocument()
    expect(screen.queryByText('可执行文件')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /返回/ }))
    expect(await screen.findByPlaceholderText('搜索 CLI')).toBeInTheDocument()
    expect(screen.getByText('可执行文件')).toBeInTheDocument()
  })

  it('opens Pi Skill management as a full secondary page', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'pi',
        name: 'Pi',
        available: true,
        capabilities,
        path: '/usr/local/bin/pi',
        version: '0.50.2',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'pi',
      localVersion: '0.50.2',
      latestVersion: '0.50.2',
      updateAvailable: false,
      command: 'npm install -g @earendil-works/pi-coding-agent@latest',
      docsUrl: 'https://github.com/badlogic/pi-mono',
      ...preparedInstall,
      configDir: '/home/u/.pi/agent',
    })

    renderPanel()
    const entry = await screen.findByText('Skill 管理')
    fireEvent.click(entry.closest('button')!)

    expect(await screen.findByText('注册 /skill:name 命令')).toBeInTheDocument()
    expect(screen.queryByPlaceholderText('搜索 CLI')).not.toBeInTheDocument()
    expect(screen.queryByText('可执行文件')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /返回/ }))
    expect(await screen.findByPlaceholderText('搜索 CLI')).toBeInTheDocument()
    expect(screen.getByText('可执行文件')).toBeInTheDocument()
  })

  it('saves the official DeepSeek API key on the dsh page', async () => {
    mockDetect.mockResolvedValue([
      {
        id: 'dsh',
        name: 'DeepSeek Harness',
        available: true,
        capabilities,
        path: 'C:\\npm\\dsh.cmd',
        version: '0.1.0-rc.6',
        models: [],
        authStatus: 'ok',
      },
    ])
    mockInstallInfo.mockResolvedValue({
      agentId: 'dsh',
      localVersion: '0.1.0-rc.6',
      latestVersion: '0.1.0-rc.6',
      updateAvailable: false,
      command: 'npm install -g @deepseek-ai/dsh@latest',
      docsUrl: 'https://github.com/deepseek-ai/dsh',
      ...preparedInstall,
      configDir: 'C:\\Users\\u\\.dsh',
    })

    renderPanel()
    const input = await screen.findByLabelText('DeepSeek API 密钥')
    expect(screen.getByText('未配置')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '获取密钥' })).toHaveAttribute(
      'href',
      'https://platform.deepseek.com/api_keys',
    )
    fireEvent.change(input, { target: { value: 'sk-test' } })
    fireEvent.click(screen.getByRole('button', { name: '保存密钥' }))
    await waitFor(() => {
      expect(mockOfficialKeySave).toHaveBeenCalledWith('sk-test')
    })
    expect(await screen.findByText('已配置')).toBeInTheDocument()
  })
})
