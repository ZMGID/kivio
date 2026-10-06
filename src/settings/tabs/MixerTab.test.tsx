import { useState } from 'react'
import { api, type ChatToolsConfig } from '../../api/tauri'
import { fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MixerTab } from './MixerTab'
import { makeSettings, makeProvider } from './testFixtures'
import { i18n } from '../../components/i18n'

vi.mock('../../api/tauri', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../api/tauri')>(),
  api: { reasoningEffortsForModel: vi.fn().mockResolvedValue(['low', 'medium', 'high']) },
}))

const t = i18n.zh

/**
 * 回归重点：六个模型槽位各自写对 defaultModels 的 key。
 * 全是 (key, providerId, model) 同签名调用，接错 key 类型检查不报错。
 */
function renderTab(overrides: Record<string, unknown> = {}) {
  const props = {
    settings: makeSettings({
      providers: [makeProvider()],
      chatProviderId: 'p1',
      ...overrides,
    } as never),
    t,
    lang: 'zh' as const,
    chatTools: { enabled: false, servers: [] } as never,
    hasChatProvider: true,
    onUpdateDefaultModel: vi.fn(),
    onUpdateChatTools: vi.fn(),
    onUpdateChat: vi.fn(),
  }
  render(<MixerTab {...props} />)
  return props
}

describe('MixerTab', () => {
  it('按角色保存模型与推理强度，换模型清除旧档位且保留其他角色', async () => {
    const saved = vi.fn()
    const settings = makeSettings({ providers: [makeProvider({ enabledModels: ['gpt-5.5', 'gpt-4o'] })] })
    function Harness() {
      const [tools, setTools] = useState<ChatToolsConfig>({ ...settings.chatTools, subAgentModels: {
        slow: { providerId: 'p1', model: 'gpt-5.5', thinkingLevel: 'high' },
      } })
      return <MixerTab settings={settings} t={t} lang="zh" chatTools={tools} hasChatProvider
        onUpdateDefaultModel={vi.fn()} onUpdateChat={vi.fn()}
        onUpdateChatTools={(update) => setTools((current) => {
          const next = { ...current, ...(typeof update === 'function' ? update(current) : update) }
          saved(next)
          return next
        })} />
    }
    render(<Harness />)
    const row = screen.getByText('SMOL').closest('.kv-row') as HTMLElement
    await userEvent.click(within(row).getByRole('button', { name: '跟随 TASK' }))
    await userEvent.click(screen.getByRole('option', { name: /gpt-5.5/ }))
    const effort = await within(row).findByRole('button', { name: 'SMOL 推理强度' })
    await userEvent.click(effort)
    await userEvent.click(screen.getByRole('option', { name: 'medium' }))
    expect(saved.mock.lastCall?.[0].subAgentModels.smol).toEqual({ providerId: 'p1', model: 'gpt-5.5', thinkingLevel: 'medium' })
    expect(saved.mock.lastCall?.[0].subAgentModels.slow.thinkingLevel).toBe('high')
    await userEvent.click(within(row).getByRole('button', { name: /gpt-5.5/ }))
    await userEvent.click(screen.getByRole('option', { name: /gpt-4o/ }))
    expect(saved.mock.lastCall?.[0].subAgentModels.smol).toEqual({ providerId: 'p1', model: 'gpt-4o', thinkingLevel: null })
    await userEvent.click(within(row).getByRole('button', { name: /gpt-4o/ }))
    await userEvent.click(screen.getByRole('option', { name: '跟随 TASK' }))
    expect(within(row).queryByRole('button', { name: 'SMOL 推理强度' })).toBeNull()
    expect(saved.mock.lastCall?.[0].subAgentModels.smol.providerId).toBe('')
  })

  it('模型没有可调推理档位时禁用推理选择', async () => {
    vi.mocked(api.reasoningEffortsForModel).mockResolvedValueOnce([])
    const settings = makeSettings()
    render(<MixerTab settings={settings} t={t} lang="zh" hasChatProvider
      chatTools={{ ...settings.chatTools, subAgentModels: { task: { providerId: 'p1', model: 'fixed' } } }}
      onUpdateChatTools={vi.fn()} onUpdateDefaultModel={vi.fn()} onUpdateChat={vi.fn()} />)
    expect(await screen.findByTitle('此模型不支持调整推理强度')).toBeDisabled()
  })

  it('可以关闭视频分析，且不清空已选模型', async () => {
    const props = renderTab()
    expect(screen.queryByText('启用视频分析')).toBeNull()
    const row = screen.getByText(t.videoAnalysisModel).closest('.kv-row')!
    await userEvent.click(within(row as HTMLElement).getByRole('button'))
    expect(screen.getAllByRole('option')[0]).toHaveTextContent('关闭')
    await userEvent.click(screen.getByRole('option', { name: '关闭' }))
    expect(props.onUpdateChat).toHaveBeenCalledWith({ videoAnalysisEnabled: false })
    expect(props.onUpdateDefaultModel).not.toHaveBeenCalled()
  })
  it('关闭后仍显示下拉菜单，选择自动可重新启用', async () => {
    const props = renderTab({ chat: { videoAnalysisEnabled: false } })
    const row = screen.getByText(t.videoAnalysisModel).closest('.kv-row')!
    await userEvent.click(within(row as HTMLElement).getByRole('button', { name: '关闭' }))
    await userEvent.click(screen.getByRole('option', { name: t.mixerAutoVisionModel }))
    expect(props.onUpdateChat).toHaveBeenCalledWith({ videoAnalysisEnabled: true })
    expect(props.onUpdateDefaultModel).toHaveBeenCalledWith('videoAnalysis', '', '')
  })
  it('视频分析只列出支持视频的模型，选择后写入独立槽位', async () => {
    const props = renderTab({ providers: [makeProvider({
      name: 'Relay',
      apiFormat: 'openai_responses',
      enabledModels: ['custom-video', 'image-only'],
      modelOverrides: {
        'custom-video': { capabilities: { videoInput: true } },
        'image-only': { capabilities: { vision: true, videoInput: false } },
      },
    })] })
    const row = screen.getByText(t.videoAnalysisModel).closest('.kv-row')!
    await userEvent.click(within(row as HTMLElement).getByRole('button'))
    expect(screen.queryByRole('option', { name: /image-only/ })).toBeNull()
    await userEvent.click(screen.getByRole('option', { name: /custom-video/ }))
    expect(props.onUpdateDefaultModel).toHaveBeenCalledWith('videoAnalysis', 'p1', 'custom-video')
  })

  it('渲染三个分组', () => {
    renderTab()
    expect(screen.getByText(t.mixerSection)).toBeTruthy()
    expect(screen.getByText(t.defaultPromptOptimizeModel)).toBeTruthy()
    expect(screen.getByText(t.mixerSubAgentSection)).toBeTruthy()
    expect(screen.getByText(t.mixerAdvisorSection)).toBeTruthy()
  })

  it('「全部恢复自动」一次重置六个槽位（不含 advisor）', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.mixerResetAuto }))
    const keys = props.onUpdateDefaultModel.mock.calls.map((c) => c[0])
    expect(keys).toEqual(['vision', 'videoAnalysis', 'titleSummary', 'compression', 'imageGeneration', 'promptOptimize'])
    // advisor 有独立开关，不该被批量重置清掉
    expect(keys).not.toContain('advisor')
  })

  it('未配供应商时显示引导文案', () => {
    renderTab()
    expect(screen.queryByText(/请先在「模型」中添加并配置供应商/)).toBeNull()
    render(
      <MixerTab
        settings={makeSettings({ providers: [] }) as never}
        t={t}
        lang="zh"
        chatTools={{ enabled: false, servers: [] } as never}
        hasChatProvider={false}
        onUpdateDefaultModel={vi.fn()}
        onUpdateChatTools={vi.fn()}
        onUpdateChat={vi.fn()}
      />,
    )
    expect(screen.getByText(/请先在「模型」中添加并配置供应商/)).toBeTruthy()
  })

  it('顾问开关关闭时不显示模型选择行', () => {
    renderTab()
    expect(screen.queryByText('顾问模型')).toBeNull()
  })

  it('顾问开关开启时显示模型选择行', () => {
    renderTab({
      defaultModels: {
        chat: { providerId: '', model: '' },
        vision: { providerId: '', model: '' },
        videoAnalysis: { providerId: '', model: '' },
        titleSummary: { providerId: '', model: '' },
        compression: { providerId: '', model: '' },
        imageGeneration: { providerId: '', model: '' },
        promptOptimize: { providerId: '', model: '' },
        advisor: { providerId: 'p1', model: 'gpt-4o' },
      },
    })
    expect(screen.getByText('顾问模型')).toBeTruthy()
  })

  it('打开顾问开关会落到第一个可用供应商的首个模型', async () => {
    const props = renderTab()
    const row = screen.getByText(t.defaultAdvisorModel).closest('.kv-row')!
    await userEvent.click(within(row as HTMLElement).getByRole('switch'))
    expect(props.onUpdateDefaultModel).toHaveBeenCalledWith('advisor', 'p1', 'gpt-4o')
  })

  it('优化提示词写入 chat.promptOptimizePrompt', () => {
    const props = renderTab()
    const areas = screen.getAllByRole('textbox')
    fireEvent.change(areas[areas.length - 1], { target: { value: '只改写问题' } })
    expect(props.onUpdateChat).toHaveBeenCalledWith({ promptOptimizePrompt: '只改写问题' })
  })
})
