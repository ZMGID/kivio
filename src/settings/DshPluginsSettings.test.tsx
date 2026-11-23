import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { externalCliSettingsApi, type DshPluginEntry, type DshPluginSettingsSnapshot } from '../api/externalCliSettings'
import { DshPluginsSettings } from './DshPluginsSettings'
import { dshPluginShortName } from './dshPluginNames'

vi.mock('../api/externalCliSettings', () => ({
  externalCliSettingsApi: {
    dshPluginSettingsGet: vi.fn(),
    dshPluginSettingsSave: vi.fn(),
    dshPluginInventory: vi.fn(),
    dshOpenSettingsFile: vi.fn(),
  },
}))

const snapshot: DshPluginSettingsSnapshot = {
  settingsPath: '/tmp/.dsh/settings.yaml',
  shell: {
    timeoutMs: 90000,
    maxOutputBytes: null,
    timeoutMsDefault: 120000,
    maxOutputBytesDefault: 64000,
  },
  agentLoop: {
    maxParallelToolCalls: null,
    maxParallelToolCallsDefault: 10,
  },
  webSearch: {
    baseUrl: null,
    maxUses: null,
    apiKeyEnv: 'DEEPSEEK_API_KEY',
    apiKeyConfigured: false,
    apiKeyWritable: true,
    baseUrlDefault: 'https://api.deepseek.com/anthropic/v1',
    maxUsesDefault: 5,
  },
}

describe('dshPluginShortName', () => {
  it('strips official package prefixes the same way dsh web does', () => {
    expect(dshPluginShortName('@deepseek-ai/cordis-plugin-include')).toBe('include')
    expect(dshPluginShortName('@deepseek-ai/dsh-timer')).toBe('timer')
    expect(dshPluginShortName('@deepseek-ai/dsh-host-plugin-inventory')).toBe('plugin-inventory')
  })
})

describe('DshPluginsSettings', () => {
  beforeEach(() => {
    vi.mocked(externalCliSettingsApi.dshPluginSettingsGet).mockReset()
    vi.mocked(externalCliSettingsApi.dshPluginSettingsSave).mockReset()
    vi.mocked(externalCliSettingsApi.dshPluginInventory).mockReset()
    vi.mocked(externalCliSettingsApi.dshOpenSettingsFile).mockReset()
    vi.mocked(externalCliSettingsApi.dshPluginSettingsGet).mockResolvedValue(snapshot)
    vi.mocked(externalCliSettingsApi.dshPluginInventory).mockResolvedValue([
      { id: 'timer', moduleName: '@deepseek-ai/dsh-timer', enabled: true },
      { id: 'tool-bash', moduleName: '@deepseek-ai/dsh-tool-bash', enabled: false },
    ])
  })

  it('refreshes the visible plugin list after a pending shell override saves', async () => {
    const user = userEvent.setup()
    let finishSave: (result: DshPluginSettingsSnapshot) => void = () => {}
    vi.mocked(externalCliSettingsApi.dshPluginSettingsSave)
      .mockImplementation(() => new Promise(resolve => { finishSave = resolve }))
    render(<DshPluginsSettings lang="zh" />)
    await waitFor(() => expect(screen.getByText('终端')).toBeInTheDocument())
    expect(screen.getByText('Agent 循环')).toBeInTheDocument()
    expect(screen.getByText('网页搜索')).toBeInTheDocument()
    expect(screen.getByText('已覆盖')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '恢复默认' })).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: '插件列表' }))
    expect(await screen.findByText('timer')).toBeInTheDocument()
    await user.click(screen.getByRole('tab', { name: '插件配置' }))
    const timeout = await screen.findByRole('textbox', { name: '命令超时（毫秒）' })
    await user.clear(timeout)
    await user.type(timeout, '45000')
    await waitFor(() => expect(timeout).toHaveValue('45000'))
    await user.click(await screen.findByRole('button', { name: '保存' }))
    await waitFor(() =>
      expect(externalCliSettingsApi.dshPluginSettingsSave).toHaveBeenCalledWith({
        shell: { timeoutMs: 45000, maxOutputBytes: null },
      }),
    )

    await user.click(screen.getByRole('tab', { name: '插件列表' }))
    expect(await screen.findByText('timer')).toBeInTheDocument()

    let finishOldInventory: (entries: DshPluginEntry[]) => void = () => {}
    vi.mocked(externalCliSettingsApi.dshPluginInventory).mockImplementationOnce(
      () => new Promise(resolve => { finishOldInventory = resolve }),
    )
    await user.click(screen.getByRole('button', { name: '重试' }))
    vi.mocked(externalCliSettingsApi.dshPluginInventory).mockResolvedValue([
      { id: 'updated', moduleName: '@deepseek-ai/dsh-updated', enabled: true },
    ])
    await act(async () => finishSave({ ...snapshot, shell: { ...snapshot.shell, timeoutMs: 45000 } }))
    expect(await screen.findByText('updated')).toBeInTheDocument()
    await act(async () => finishOldInventory([{ id: 'timer', moduleName: '@deepseek-ai/dsh-timer', enabled: true }]))
    expect(screen.getByText('updated')).toBeInTheDocument()
    expect(screen.queryByText('timer')).not.toBeInTheDocument()
  })

  it('filters composed plugins, reuses them across tabs, and refreshes on request', async () => {
    render(<DshPluginsSettings lang="zh" />)
    fireEvent.click(screen.getByRole('tab', { name: '插件列表' }))
    await waitFor(() => expect(screen.getByText('timer')).toBeInTheDocument())
    expect(screen.getByText('已启用')).toBeInTheDocument()
    expect(screen.getByText('已停用')).toBeInTheDocument()
    expect(screen.getByText('2')).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText('搜索插件'), { target: { value: 'bash' } })
    expect(screen.queryByText('timer')).not.toBeInTheDocument()
    expect(screen.getByText('tool-bash')).toBeInTheDocument()
    expect(screen.getByText('1')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('tab', { name: '插件配置' }))
    fireEvent.click(screen.getByRole('tab', { name: '插件列表' }))
    expect(screen.getByText('timer')).toBeInTheDocument()
    expect(externalCliSettingsApi.dshPluginInventory).toHaveBeenCalledTimes(1)

    let rejectRefresh: (reason: string) => void = () => {}
    vi.mocked(externalCliSettingsApi.dshPluginInventory)
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectRefresh = reject }))
      .mockResolvedValueOnce([])
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(screen.getByText('timer')).toBeInTheDocument()
    await act(async () => rejectRefresh('dsh 读取超时'))
    expect(screen.getByRole('alert')).toHaveTextContent('暂时无法读取插件。')
    expect(screen.getByText('dsh 读取超时')).toBeInTheDocument()
    expect(screen.getByText('timer')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(await screen.findByText('暂无插件。')).toBeInTheDocument()
    expect(externalCliSettingsApi.dshPluginInventory).toHaveBeenCalledTimes(3)
  })

  it('surfaces the inventory error detail instead of swallowing it', async () => {
    vi.mocked(externalCliSettingsApi.dshPluginInventory).mockRejectedValue(
      'dsh --dump-config 失败：profile missing',
    )
    render(<DshPluginsSettings lang="zh" />)
    fireEvent.click(screen.getByRole('tab', { name: '插件列表' }))
    await waitFor(() => expect(screen.getByText('暂时无法读取插件。')).toBeInTheDocument())
    expect(screen.getByText('dsh --dump-config 失败：profile missing')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument()
  })
})
