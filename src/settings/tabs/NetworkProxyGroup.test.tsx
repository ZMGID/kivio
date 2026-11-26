import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NetworkProxyGroup } from './GeneralTab'
import type { CustomProxyDraft } from './GeneralTab'
import { makeSettings } from './testFixtures'
import { api } from '../../api/tauri'
import { notifyError } from '../../components/appNoticeStore'
import type { NetworkProxySettings } from '../../api/tauri'

vi.mock('../../api/tauri', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../api/tauri')>()
  return { ...actual, api: { ...actual.api, networkProxyTest: vi.fn() } }
})
vi.mock('../../components/appNoticeStore', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../components/appNoticeStore')>()
  return { ...actual, notifyError: vi.fn() }
})

function renderGroup(networkProxy?: NetworkProxySettings) {
  const onUpdateSettings = vi.fn()
  function Group() {
    const [settings, setSettings] = useState(() => makeSettings(networkProxy ? { networkProxy } : {}))
    const [draft, setDraft] = useState<CustomProxyDraft | null>(null)
    return <NetworkProxyGroup settings={settings} lang="zh" customDraft={draft} onCustomDraftChange={setDraft}
      onUpdateSettings={(updates) => {
        onUpdateSettings(updates)
        setSettings((previous) => ({ ...previous, ...updates }))
      }}
    />
  }
  render(<Group />)
  return { onUpdateSettings }
}

describe('NetworkProxyGroup', () => {
  beforeEach(() => vi.clearAllMocks())

  it('defaults to the system proxy for snapshots without the setting', () => {
    renderGroup()
    expect(screen.getByRole('button', { name: '代理模式' })).toHaveTextContent('系统代理')
    expect(screen.queryByLabelText('代理地址')).not.toBeInTheDocument()
  })

  it('keeps custom mode open without saving an empty address', async () => {
    const { onUpdateSettings } = renderGroup()
    await userEvent.click(screen.getByRole('button', { name: '代理模式' }))
    await userEvent.click(screen.getByRole('option', { name: '自定义代理' }))
    expect(onUpdateSettings).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: '代理模式' })).toHaveTextContent('自定义代理')
    expect(screen.getByLabelText('代理地址')).toHaveValue('')
    expect(screen.getByRole('button', { name: '保存代理' })).toBeDisabled()
  })

  it('saves the custom mode, address and bypass list together', async () => {
    const { onUpdateSettings } = renderGroup()
    await userEvent.click(screen.getByRole('button', { name: '代理模式' }))
    await userEvent.click(screen.getByRole('option', { name: '自定义代理' }))
    const address = screen.getByLabelText('代理地址')
    fireEvent.change(address, { target: { value: 'socks5://127.0.0.1:1080' } })
    fireEvent.change(screen.getByLabelText('不走代理的地址'), { target: { value: '*.corp.local' } })
    expect(onUpdateSettings).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '保存代理' }))
    expect(onUpdateSettings).toHaveBeenLastCalledWith({
      networkProxy: { mode: 'custom', url: 'socks5://127.0.0.1:1080', bypass: '*.corp.local' },
    })
    expect(screen.getByRole('button', { name: '代理模式' })).toHaveTextContent('自定义代理')
    expect(address).toHaveValue('socks5://127.0.0.1:1080')
  })

  it('flags malformed addresses and blocks testing or saving them', () => {
    renderGroup({ mode: 'custom', url: 'http://127.0.0.1:7890', bypass: '' })
    fireEvent.change(screen.getByLabelText('代理地址'), { target: { value: 'http://127.0.0.1:99999' } })
    expect(screen.getByText('格式应为 协议://主机:端口')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '测试' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '保存代理' })).toBeDisabled()
  })

  it('tests the unsaved draft and reports success inline, failure as a notice', async () => {
    const draft = { mode: 'custom' as const, url: 'http://127.0.0.1:7890', bypass: '' }
    vi.mocked(api.networkProxyTest).mockResolvedValueOnce({ latencyMs: 42, status: 204 })
    renderGroup(draft)
    await userEvent.click(screen.getByRole('button', { name: '测试' }))
    expect(api.networkProxyTest).toHaveBeenCalledWith(draft)
    expect(await screen.findByText('连接成功 · 42 ms')).toBeInTheDocument()

    vi.mocked(api.networkProxyTest).mockRejectedValueOnce('Connection failed: refused')
    await userEvent.click(screen.getByRole('button', { name: '测试' }))
    await vi.waitFor(() => expect(notifyError).toHaveBeenCalledWith('代理测试失败: Connection failed: refused'))
    expect(screen.queryByText('连接成功 · 42 ms')).not.toBeInTheDocument()
  })
})
