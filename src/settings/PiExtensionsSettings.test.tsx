import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { externalCliSettingsApi, type PiExtensionInventory } from '../api/externalCliSettings'
import { PiExtensionsSettings } from './PiExtensionsSettings'
import { resetPiExtensionsOperationState } from './piExtensionsOperation'
import { open } from '@tauri-apps/plugin-dialog'

vi.mock('../api/externalCliSettings', () => ({
  externalCliSettingsApi: {
    piExtensionsInventory: vi.fn(),
    piExtensionSetEnabled: vi.fn(),
    piExtensionInstall: vi.fn(),
    piExtensionUpdate: vi.fn(),
    piExtensionRemove: vi.fn(),
    piExtensionOpen: vi.fn(),
    piExtensionsOpenDir: vi.fn(),
  },
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
const inventory: PiExtensionInventory = {
  agentDir: '/home/u/.pi/agent',
  extensionsDir: '/home/u/.pi/agent/extensions',
  packages: [
    {
      source: 'npm:pi-mcp-adapter',
      name: 'pi-mcp-adapter',
      version: '2.25.0',
      description: 'MCP adapter extension',
      path: '/home/u/.pi/agent/npm/node_modules/pi-mcp-adapter',
      enabled: true,
      canToggle: true,
      hasExtensions: true,
      extensionEntries: 1,
      resources: ['extensions', 'skills'],
    },
    {
      source: 'npm:pi-curated-themes',
      name: 'pi-curated-themes',
      version: '0.2.1',
      description: 'Themes and skills',
      path: '/home/u/.pi/agent/npm/node_modules/pi-curated-themes',
      enabled: true,
      canToggle: false,
      hasExtensions: false,
      extensionEntries: 0,
      resources: ['skills', 'themes'],
    },
    {
      source: 'npm:custom-filtered',
      name: 'custom-filtered',
      version: '1.0.0',
      description: null,
      path: '/home/u/.pi/agent/npm/node_modules/custom-filtered',
      enabled: true,
      canToggle: false,
      hasExtensions: true,
      extensionEntries: 2,
      resources: ['extensions'],
    },
  ],
  localExtensions: [
    {
      relativePath: 'local-tool.ts',
      name: 'local-tool',
      path: '/home/u/.pi/agent/extensions/local-tool.ts',
      enabled: false,
      kind: 'file',
    },
  ],
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

describe('PiExtensionsSettings', () => {
  beforeEach(() => {
    resetPiExtensionsOperationState()
    vi.mocked(externalCliSettingsApi.piExtensionsInventory).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionSetEnabled).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionInstall).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionUpdate).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionRemove).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionOpen).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionsOpenDir).mockReset()
    vi.mocked(open).mockReset()
    vi.mocked(externalCliSettingsApi.piExtensionsInventory).mockResolvedValue(inventory)
    vi.mocked(externalCliSettingsApi.piExtensionSetEnabled).mockResolvedValue()
    vi.mocked(externalCliSettingsApi.piExtensionInstall).mockResolvedValue({ output: 'installed' })
    vi.mocked(externalCliSettingsApi.piExtensionUpdate).mockResolvedValue({ output: 'updated' })
    vi.mocked(externalCliSettingsApi.piExtensionRemove).mockResolvedValue({ output: 'removed' })
  })

  it('lists packages and local extensions with safe toggle states', async () => {
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)

    const packageRow = (await screen.findByText('pi-mcp-adapter')).closest<HTMLElement>('.kv-row')!
    expect(within(packageRow).getByText('v2.25.0')).toBeInTheDocument()
    expect(within(packageRow).getByRole('switch')).toHaveAttribute('aria-checked', 'true')

    const resourceRow = screen.getByText('pi-curated-themes').closest<HTMLElement>('.kv-row')!
    expect(within(resourceRow).getByText('资源包')).toBeInTheDocument()
    expect(within(resourceRow).queryByRole('switch')).not.toBeInTheDocument()

    const filteredRow = screen.getByText('custom-filtered').closest<HTMLElement>('.kv-row')!
    expect(within(filteredRow).getByText('pi config')).toBeInTheDocument()
    expect(within(filteredRow).queryByRole('switch')).not.toBeInTheDocument()

    const localRow = screen.getByText('local-tool').closest<HTMLElement>('.kv-row')!
    expect(within(localRow).getByRole('switch')).toHaveAttribute('aria-checked', 'false')
  })

  it('toggles extensions and installs a package source', async () => {
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)

    const packageRow = (await screen.findByText('pi-mcp-adapter')).closest<HTMLElement>('.kv-row')!
    fireEvent.click(within(packageRow).getByRole('switch'))
    await waitFor(() => {
      expect(externalCliSettingsApi.piExtensionSetEnabled).toHaveBeenCalledWith(
        'package',
        'npm:pi-mcp-adapter',
        false,
      )
    })

    const source = screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')
    fireEvent.change(source, { target: { value: 'npm:example-extension' } })
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await waitFor(() => {
      expect(externalCliSettingsApi.piExtensionInstall).toHaveBeenCalledWith('npm:example-extension')
    })
  })

  it('starts the local package picker from the Pi global directory', async () => {
    vi.mocked(open).mockResolvedValue('C:\\packages\\demo')
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)

    await screen.findByText('pi-mcp-adapter')
    fireEvent.click(screen.getByRole('button', { name: '选择本地 Package 目录' }))

    await waitFor(() => {
      expect(open).toHaveBeenCalledWith({
        multiple: false,
        directory: true,
        defaultPath: inventory.agentDir,
      })
    })
    expect(screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')).toHaveValue(
      'C:\\packages\\demo',
    )
  })

  it('starts an install once and restores a failure with the draft source after leaving', async () => {
    const gate = deferred<{ output: string }>()
    vi.mocked(externalCliSettingsApi.piExtensionInstall).mockReturnValue(gate.promise)
    let view = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    await screen.findByText('pi-mcp-adapter')
    const source = screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')
    const search = screen.getByPlaceholderText('搜索扩展或来源')
    fireEvent.change(source, { target: { value: 'npm:draft-extension' } })
    fireEvent.change(search, { target: { value: 'mcp' } })
    const install = screen.getByRole('button', { name: '安装' })
    await act(async () => {
      install.click()
      install.click()
    })
    expect(externalCliSettingsApi.piExtensionInstall).toHaveBeenCalledOnce()
    expect(externalCliSettingsApi.piExtensionInstall).toHaveBeenCalledWith('npm:draft-extension')
    expect(screen.getByRole('button', { name: '安装中…' })).toBeDisabled()

    view.unmount()
    view = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    expect(screen.getByRole('button', { name: '安装中…' })).toBeDisabled()
    expect(screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')).toHaveValue('npm:draft-extension')
    expect(screen.getByPlaceholderText('搜索扩展或来源')).toHaveValue('mcp')
    view.unmount()

    await act(async () => { gate.reject(new Error('registry down')) })
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent('registry down')
    expect(screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')).toHaveValue('npm:draft-extension')
    expect(screen.getByPlaceholderText('搜索扩展或来源')).toHaveValue('mcp')
    expect(externalCliSettingsApi.piExtensionInstall).toHaveBeenCalledOnce()
  })

  it('restores an in-flight update and the refreshed inventory after the page is gone', async () => {
    const gate = deferred<{ output: string }>()
    const refreshed: PiExtensionInventory = {
      ...inventory,
      packages: [
        ...inventory.packages,
        { ...inventory.packages[0], source: 'npm:fresh-pack', name: 'fresh-pack' },
      ],
    }
    let refresh = false
    vi.mocked(externalCliSettingsApi.piExtensionUpdate).mockReturnValue(gate.promise)
    vi.mocked(externalCliSettingsApi.piExtensionsInventory).mockImplementation(async () => (
      refresh ? refreshed : inventory
    ))
    let view = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    await screen.findByText('pi-mcp-adapter')
    fireEvent.click(screen.getByRole('button', { name: '全部更新' }))
    await waitFor(() => expect(externalCliSettingsApi.piExtensionUpdate).toHaveBeenCalledOnce())
    expect(screen.getByRole('button', { name: '更新中…' })).toBeDisabled()
    view.unmount()

    view = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    expect(screen.getByRole('button', { name: '更新中…' })).toBeDisabled()
    view.unmount()

    refresh = true
    await act(async () => { gate.resolve({ output: 'updated all' }) })
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    expect(await screen.findByText('updated all')).toBeInTheDocument()
    expect(screen.getByText('fresh-pack')).toBeInTheDocument()
    expect(externalCliSettingsApi.piExtensionUpdate).toHaveBeenCalledOnce()
  })

  it('restores a package removal failure that finishes while the page is gone', async () => {
    vi.stubGlobal('confirm', () => true)
    const gate = deferred<{ output: string }>()
    vi.mocked(externalCliSettingsApi.piExtensionRemove).mockReturnValue(gate.promise)
    const view = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    const row = (await screen.findByText('pi-mcp-adapter')).closest<HTMLElement>('.kv-row')!
    fireEvent.click(within(row).getByRole('button', { name: '卸载 Package' }))
    await waitFor(() => {
      expect(externalCliSettingsApi.piExtensionRemove).toHaveBeenCalledWith('npm:pi-mcp-adapter')
    })
    view.unmount()
    await act(async () => { gate.reject(new Error('package is locked')) })
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent('package is locked')
    expect(externalCliSettingsApi.piExtensionRemove).toHaveBeenCalledOnce()
  })
  it('does not let a retired folder picker replace a source typed after returning', async () => {
    const picker = deferred<string | null>()
    vi.mocked(open).mockReturnValue(picker.promise)
    const first = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    await screen.findByText('pi-mcp-adapter')
    fireEvent.click(screen.getByRole('button', { name: '选择本地 Package 目录' }))
    first.unmount()
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    const source = screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')
    fireEvent.change(source, { target: { value: 'npm:new-source' } })
    await act(async () => { picker.resolve('/retired/package'); await picker.promise })
    expect(source).toHaveValue('npm:new-source')
  })

  it.each(['unchanged', 'replacement'])('clears only the submitted source after a successful install with %s input', async (input) => {
    const gate = deferred<{ output: string }>()
    vi.mocked(externalCliSettingsApi.piExtensionInstall).mockReturnValue(gate.promise)
    const first = render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    await screen.findByText('pi-mcp-adapter')
    fireEvent.change(screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径'), { target: { value: 'npm:submitted' } })
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '安装中…' })).toBeDisabled())
    first.unmount()
    render(<PiExtensionsSettings lang="zh" onBack={vi.fn()} />)
    const source = screen.getByPlaceholderText('npm:包名、git:仓库地址或本地路径')
    if (input === 'replacement') fireEvent.change(source, { target: { value: 'npm:new-draft' } })
    await act(async () => { gate.resolve({ output: 'installed submitted' }) })
    expect(source).toHaveValue(input === 'replacement' ? 'npm:new-draft' : '')
    expect(await screen.findByText('installed submitted')).toBeInTheDocument()
    if (input === 'replacement') expect(screen.getByRole('button', { name: '安装' })).toBeEnabled()
    else expect(screen.getByRole('button', { name: '安装' })).toBeDisabled()
  })
})
