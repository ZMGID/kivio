import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AgentModelPicker, type AgentModelPickerProps } from './AgentModelPicker'
import { LAST_AGENT_RUNTIME_KEY } from './lastAgentRuntime'
import type { AgentRuntimeConfig } from './types'
import type { ExternalAgentModels } from '../api/externalCliSettings'

const detectModels = vi.fn()
const detectAgents = vi.fn()
const { thinkingCapabilitiesForModel } = vi.hoisted(() => ({ thinkingCapabilitiesForModel: vi.fn() }))

vi.mock('./api', () => ({
  chatApi: {
    detectExternalAgents: (...args: unknown[]) => detectAgents(...args),
    detectExternalAgentModels: (...args: unknown[]) => detectModels(...args),
  },
}))
vi.mock('../api/tauri', () => ({
  api: { getSettings: () => Promise.resolve({ providers: [] }), thinkingCapabilitiesForModel },
}))
vi.mock('../api/settingsCache', () => ({
  getSettingsCached: async () => ({
    providers: [{ id: 'p1', name: 'Provider One', enabled: true, enabledModels: ['m1', 'm2'], availableModels: [] }],
    favoriteModels: [],
  }),
  subscribeSettings: () => () => {},
  setFavoriteModelsCached: vi.fn(),
}))
// 'no-brand-model' 代表认不出品牌的模型名。
vi.mock('../components/ModelIcon', () => ({ ModelIcon: () => null, hasModelIcon: (model: string) => model !== 'no-brand-model' }))

const runtime: AgentRuntimeConfig = {
  kind: 'external',
  externalAgentId: 'cursor',
  externalModel: 'default',
  externalReasoning: null,
}
const builtin: AgentRuntimeConfig = { kind: 'builtin', externalAgentId: null, externalModel: null, externalReasoning: null }

function renderPicker(props: Partial<AgentModelPickerProps> = {}) {
  const all: AgentModelPickerProps = {
    agentRuntime: builtin,
    providerId: 'p1',
    model: 'm1',
    thinkingLevel: null,
    onRuntimeChange: vi.fn(),
    onModelChange: vi.fn(),
    onExternalModelChange: vi.fn(),
    onThinkingLevelChange: vi.fn(),
    ...props,
  }
  const view = render(<AgentModelPicker {...all} />)
  return {
    ...view,
    props: all,
    rerenderWith: (next: Partial<AgentModelPickerProps>) => view.rerender(<AgentModelPicker {...all} {...next} />),
  }
}

// jsdom 没有 PointerEvent：用 MouseEvent 补上，滑杆拖动测试才能带 clientX。
if (typeof window.PointerEvent === 'undefined') {
  class PointerEventPolyfill extends MouseEvent {
    pointerId: number
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init)
      this.pointerId = init.pointerId ?? 0
    }
  }
  Object.defineProperty(window, 'PointerEvent', { value: PointerEventPolyfill, configurable: true })
}

const trigger = () => document.querySelector<HTMLButtonElement>('.kv-agent-picker > button')!
const openPicker = () => act(() => { fireEvent.click(trigger()) })
const panel = () => screen.getByRole('dialog')

beforeEach(() => {
  detectModels.mockReset()
  detectAgents.mockReset()
  detectAgents.mockResolvedValue([])
  thinkingCapabilitiesForModel.mockReset()
  thinkingCapabilitiesForModel.mockResolvedValue({ levels: ['low', 'medium', 'high'], offMode: 'supported' })
})

describe('AgentModelPicker · external CLI models and effort', () => {
  it('OpenCode exposes each model’s variants, supports Auto, and clears effort for models without variants', async () => {
    const onExternalModelChange = vi.fn()
    const selected: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'opencode', externalModel: 'custom/reasoner', externalReasoning: 'low' }
    detectModels.mockResolvedValue({
      models: [{ id: 'custom/reasoner', label: 'custom/reasoner' }, { id: 'custom/plain', label: 'custom/plain' }],
      source: 'probed', reasoningOptions: [],
      reasoningByModel: {
        'custom/reasoner': [{ id: 'default', label: 'Default' }, { id: 'low', label: 'Low' }, { id: 'custom-budget', label: 'Custom budget' }],
        'custom/plain': [],
      },
    })
    const view = renderPicker({ agentRuntime: selected, onExternalModelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('Low'))
    openPicker()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Custom budget' }))
    expect(onExternalModelChange).toHaveBeenLastCalledWith('custom/reasoner', 'custom-budget')
    // 调强度不关闭弹层，方便继续设置。
    expect(panel()).toBeInTheDocument()
    view.rerenderWith({ agentRuntime: { ...selected, externalReasoning: 'custom-budget' } })
    // CLI 默认不占滑杆落点，也不再放「恢复默认」按钮。
    expect(screen.queryByRole('menuitemradio', { name: 'Default' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '恢复默认强度' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'custom/plain' }))
    expect(onExternalModelChange).toHaveBeenLastCalledWith('custom/plain', null)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    view.rerenderWith({ agentRuntime: { ...selected, externalModel: 'custom/plain', externalReasoning: null } })
    expect(trigger().querySelector('.chat-thinking-level-label')).toBeNull()
    openPicker()
    expect(screen.queryByRole('menuitemradio')).not.toBeInTheDocument()
  })

  it('换到还在探测的 CLI：弹层定高、底栏先占位，探测提示稍等才出现，不一闪而过', async () => {
    detectModels.mockImplementation(() => new Promise(() => {}))
    renderPicker({ agentRuntime: runtime })
    openPicker()
    expect(panel().style.height).not.toBe('')
    expect(panel().querySelector('.kv-effort__track.is-placeholder')).not.toBeNull()
    expect(screen.queryByText('正在探测模型…')).not.toBeInTheDocument()
    expect(await screen.findByText('正在探测模型…')).toBeInTheDocument()
  })

  it('shows saved models during refresh without restoring a stale selection or overwriting a user choice', async () => {
    const onExternalModelChange = vi.fn()
    let finish: (value: ExternalAgentModels) => void = () => {}
    detectModels.mockImplementation((_agent, _conversation, _force, onSnapshot) => {
      onSnapshot({ models: [{ id: 'saved', label: 'Saved' }], reasoningOptions: [], source: 'probed', currentModel: 'saved', refreshing: true })
      return new Promise<ExternalAgentModels>((resolve) => { finish = resolve })
    })
    renderPicker({ agentRuntime: runtime, onExternalModelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('Saved'))
    openPicker()
    expect(screen.queryByText('正在探测模型…')).not.toBeInTheDocument()
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(onExternalModelChange).not.toHaveBeenCalled()
    fireEvent.click(within(panel()).getByRole('button', { name: 'Saved' }))
    expect(onExternalModelChange).toHaveBeenCalledWith('saved', null)
    await act(async () => finish({ models: [{ id: 'new', label: 'New' }], reasoningOptions: [], reasoningByModel: {}, source: 'probed', currentModel: 'new' }))
    expect(onExternalModelChange).toHaveBeenCalledTimes(1)
  })

  it('discards both snapshot and refresh callbacks after unmount', async () => {
    const onExternalModelChange = vi.fn()
    let snapshot: (value: ExternalAgentModels) => void = () => {}
    let finish: (value: ExternalAgentModels) => void = () => {}
    detectModels.mockImplementation((_agent, _conversation, _force, onSnapshot) => {
      snapshot = onSnapshot
      return new Promise<ExternalAgentModels>((resolve) => { finish = resolve })
    })
    const view = renderPicker({ agentRuntime: runtime, onExternalModelChange })
    view.unmount()
    const result: ExternalAgentModels = { models: [{ id: 'late', label: 'Late' }], reasoningOptions: [], reasoningByModel: {}, source: 'probed', currentModel: 'late' }
    await act(async () => { snapshot({ ...result, refreshing: true }); finish(result) })
    expect(onExternalModelChange).not.toHaveBeenCalled()
  })

  it('clears the old catalog when changing conversations and ignores its late refresh', async () => {
    const onExternalModelChange = vi.fn()
    let finishOld: (value: ExternalAgentModels) => void = () => {}
    detectModels.mockImplementation((_agent, conversation, _force, onSnapshot) => {
      if (conversation === 'old') {
        onSnapshot({ models: [{ id: 'old', label: 'Old project' }], reasoningOptions: [], source: 'probed', currentModel: 'old', refreshing: true })
        return new Promise<ExternalAgentModels>((resolve) => { finishOld = resolve })
      }
      return new Promise<ExternalAgentModels>(() => {})
    })
    const view = renderPicker({ agentRuntime: runtime, conversationId: 'old', onExternalModelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('Old project'))
    view.rerenderWith({ conversationId: 'new' })
    openPicker()
    expect(screen.queryByText('Old project')).not.toBeInTheDocument()
    await act(async () => finishOld({ models: [{ id: 'late', label: 'Late' }], reasoningOptions: [], reasoningByModel: {}, source: 'probed', currentModel: 'late' }))
    expect(screen.queryByText('Late')).not.toBeInTheDocument()
    expect(onExternalModelChange).not.toHaveBeenCalled()
  })

  it('toggles the catalog fast tier without changing model or effort', async () => {
    const onExternalModelChange = vi.fn()
    detectModels.mockResolvedValue({ models: [{ id: 'gpt-test', label: 'Test model', capabilities: { serviceTiers: [{ id: 'priority', label: 'Fast' }] } }],
      reasoningOptions: [], source: 'probed' })
    const codex: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'codex', externalModel: 'gpt-test', externalReasoning: 'high' }
    const view = renderPicker({ agentRuntime: codex, onExternalModelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('Test model'))
    openPicker()
    fireEvent.click(await screen.findByRole('button', { name: '快速模式' }))
    expect(onExternalModelChange).toHaveBeenLastCalledWith('gpt-test', undefined, 'priority')
    view.rerenderWith({ agentRuntime: { ...codex, externalServiceTier: 'priority' } })
    expect(trigger()).toHaveAccessibleName(/快速模式/)
    fireEvent.click(screen.getByRole('button', { name: '快速模式' }))
    expect(onExternalModelChange).toHaveBeenLastCalledWith('gpt-test', undefined, null)
  })

  it('Antigravity 保留档位名称，清除旧覆盖，切换变体时不携带 effort', async () => {
    const onExternalModelChange = vi.fn()
    detectModels.mockResolvedValue({
      models: [
        { id: 'gemini-3.8-flash-high', label: 'Gemini 3.8 Flash (High)' },
        { id: 'gemini-3.8-flash-low', label: 'Gemini 3.8 Flash (Low)' },
      ],
      reasoningOptions: [{ id: 'high', label: 'High' }, { id: 'low', label: 'Low' }],
      source: 'probed',
    })
    renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'antigravity', externalModel: 'gemini-3.8-flash-high', externalReasoning: 'low' }, onExternalModelChange })
    await waitFor(() => expect(onExternalModelChange).toHaveBeenCalledWith('gemini-3.8-flash-high', null))
    expect(trigger()).toHaveTextContent('Gemini 3.8 Flash (High)')
    openPicker()
    expect(screen.queryByRole('menuitemradio')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Gemini 3.8 Flash (Low)' }))
    expect(onExternalModelChange).toHaveBeenLastCalledWith('gemini-3.8-flash-low', null)
  })

  it('Antigravity 自动探测档位模型时不回填独立 effort', async () => {
    const onExternalModelChange = vi.fn()
    detectModels.mockResolvedValue({
      models: [{ id: 'gemini-3.8-flash-low', label: 'Gemini 3.8 Flash (Low)' }],
      reasoningOptions: [{ id: 'high', label: 'High' }],
      currentModel: 'gemini-3.8-flash-low', currentReasoning: 'high', source: 'probed',
    })
    renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'antigravity' }, onExternalModelChange })
    await waitFor(() => expect(onExternalModelChange).toHaveBeenCalledWith('gemini-3.8-flash-low', null))
    expect(trigger().querySelector('.chat-thinking-level-label')).toBeNull()
  })

  it('探测失败降级时展示默认列表提示，重试以 force=true 重探', async () => {
    detectModels
      .mockResolvedValueOnce({ models: [{ id: 'default', label: 'Default' }], reasoningOptions: [], source: 'fallback', probeError: 'boom' })
      .mockResolvedValueOnce({ models: [{ id: 'gpt-5', label: 'gpt-5' }], reasoningOptions: [], source: 'probed' })
    renderPicker({ agentRuntime: runtime, conversationId: null })
    await waitFor(() => expect(detectModels).toHaveBeenCalledTimes(1))
    expect(detectModels.mock.calls[0][2]).toBeUndefined()
    await waitFor(() => expect(trigger()).toHaveTextContent('选择模型'))
    openPicker()
    expect(screen.getByText('刷新失败，保留当前列表')).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent('boom')
    act(() => { fireEvent.click(screen.getByText('重试')) })
    await waitFor(() => expect(detectModels).toHaveBeenCalledTimes(2))
    expect(detectModels.mock.calls[1][2]).toBe(true)
    await waitFor(() => expect(screen.queryByText('boom')).not.toBeInTheDocument())
  })

  it('probed 结果不显示降级提示', async () => {
    detectModels.mockResolvedValue({ models: [{ id: 'gpt-5', label: 'gpt-5' }], reasoningOptions: [], source: 'probed' })
    renderPicker({ agentRuntime: runtime, conversationId: null })
    await waitFor(() => expect(detectModels).toHaveBeenCalledTimes(1))
    openPicker()
    expect(screen.queryByText('刷新失败，保留当前列表')).not.toBeInTheDocument()
    expect(await screen.findByText('gpt-5')).toBeInTheDocument()
  })

  it('探测到 CLI 当前模型/推理时自动同步（未显式选择）', async () => {
    const onExternalModelChange = vi.fn()
    detectModels.mockResolvedValue({
      models: [{ id: 'default', label: 'Default' }, { id: 'grok-4.5', label: 'Grok 4.5' }],
      reasoningOptions: [{ id: 'default', label: 'Default' }, { id: 'high', label: 'High' }],
      source: 'probed', currentModel: 'grok-4.5', currentReasoning: 'high',
    })
    renderPicker({ agentRuntime: runtime, onExternalModelChange })
    await waitFor(() => expect(onExternalModelChange).toHaveBeenCalledWith('grok-4.5', 'high'))
    openPicker()
    expect(screen.getByRole('button', { name: 'Grok 4.5' })).toHaveAttribute('aria-current', 'true')
  })

  it('用户已显式选择模型时不被当前配置覆盖', async () => {
    const onExternalModelChange = vi.fn()
    detectModels.mockResolvedValue({
      models: [{ id: 'gpt-5', label: 'gpt-5' }], reasoningOptions: [], source: 'probed',
      currentModel: 'gpt-5.6-sol', currentReasoning: 'high',
    })
    renderPicker({ agentRuntime: { ...runtime, externalModel: 'gpt-5' }, onExternalModelChange })
    await waitFor(() => expect(detectModels).toHaveBeenCalled())
    await waitFor(() => expect(trigger()).toHaveTextContent('gpt-5'))
    expect(onExternalModelChange).not.toHaveBeenCalled()
  })

  it('dsh 的 off 是真档位：胶囊显示 Off，弹层里保留 Off', async () => {
    detectModels.mockResolvedValue({
      models: [{ id: 'default', label: 'Default' }, { id: 'deepseek-v4-flash', label: 'DeepSeek-V4-Flash' }],
      reasoningOptions: [{ id: 'default', label: 'Default' }, { id: 'off', label: 'Off' }, { id: 'high', label: 'High' }, { id: 'max', label: 'Max' }],
      source: 'probed', currentModel: 'deepseek-v4-flash', currentReasoning: 'off',
    })
    renderPicker({ agentRuntime: runtime })
    await waitFor(() => expect(trigger()).toHaveAccessibleName(/思考等级：Off/))
    openPicker()
    expect(screen.getByRole('menuitemradio', { name: 'Off' })).toBeInTheDocument()
    expect(trigger()).not.toHaveAccessibleName(/思考等级：Auto/)
  })

  it('ACP 只报 on/off 开关时不显示思考档位', async () => {
    detectModels.mockResolvedValue({
      models: [{ id: 'default', label: 'Default' }],
      reasoningOptions: [{ id: 'on', label: 'On' }, { id: 'off', label: 'Off' }],
      source: 'probed', currentModel: null, currentReasoning: 'off',
    })
    renderPicker({ agentRuntime: runtime })
    await waitFor(() => expect(trigger()).toHaveTextContent('选择模型'))
    expect(trigger()).not.toHaveAccessibleName(/思考等级/)
    openPicker()
    expect(screen.queryByRole('menuitemradio')).not.toBeInTheDocument()
  })

  it('有 CLI 当前模型/推理时胶囊显示真实名字而不是 Auto', async () => {
    detectModels.mockResolvedValue({
      models: [{ id: 'default', label: 'Default' }, { id: 'deepseek-v4-flash', label: 'DeepSeek-V4-Flash' }],
      reasoningOptions: [{ id: 'default', label: 'Default' }, { id: 'off', label: 'Off' }, { id: 'high', label: 'High' }, { id: 'max', label: 'Max' }],
      source: 'probed', currentModel: 'deepseek-v4-flash', currentReasoning: 'high',
    })
    renderPicker({ agentRuntime: runtime })
    await waitFor(() => expect(trigger()).toHaveTextContent('DeepSeek-V4-Flash'))
    expect(trigger()).toHaveAccessibleName(/思考等级：High/)
    expect(trigger()).not.toHaveTextContent('Auto')
  })

  it('Codex effort 营销文案只保留档位名', async () => {
    detectModels.mockResolvedValue({
      models: [{ id: 'default', label: 'Default' }, { id: 'gpt-5.6-sol', label: 'GPT-5.6-Sol' }],
      reasoningOptions: [
        { id: 'high', label: 'high — Greater reasoning depth for complex problems' },
        { id: 'ultra', label: 'ultra — Maximum reasoning with automatic task delegation' },
      ],
      source: 'probed', currentModel: 'gpt-5.6-sol', currentReasoning: 'high',
    })
    renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'codex', externalReasoning: 'high' } })
    // CLI 报的小写 id 按内置写法统一大小写。
    await waitFor(() => expect(trigger()).toHaveAccessibleName(/思考等级：High/))
    expect(trigger()).not.toHaveTextContent('Greater')
    openPicker()
    expect(screen.getByRole('menuitemradio', { name: 'Ultra' })).toBeInTheDocument()
    expect(screen.queryByText(/Greater reasoning/)).not.toBeInTheDocument()
    expect(screen.queryByText(/automatic task delegation/)).not.toBeInTheDocument()
  })

  it('无当前模型时提示选择模型并显示空目录', async () => {
    detectModels.mockResolvedValue({
      models: [], reasoningOptions: [], source: 'probed', currentModel: null, currentReasoning: null,
    })
    renderPicker({ agentRuntime: runtime })
    await waitFor(() => expect(trigger()).toHaveTextContent('选择模型'))
    openPicker()
    expect(await screen.findByText('该 CLI 未上报可用模型')).toBeInTheDocument()
  })

  it('切换 agent 时旧 CLI 的 currentModel 立即失效（不残留在胶囊上）', async () => {
    detectModels.mockImplementation((agentId: unknown) => {
      if (agentId === 'cursor') {
        return Promise.resolve({
          models: [{ id: 'default', label: 'Default' }, { id: 'grok-4.5', label: 'Grok 4.5' }],
          reasoningOptions: [], source: 'probed', currentModel: 'grok-4.5', currentReasoning: null,
        })
      }
      return new Promise(() => {})
    })
    const view = renderPicker({ agentRuntime: runtime })
    await waitFor(() => expect(trigger()).toHaveTextContent('Grok 4.5'))
    view.rerenderWith({ agentRuntime: { ...runtime, externalAgentId: 'claude', externalModel: 'default' } })
    await waitFor(() => expect(trigger()).toHaveTextContent('获取中…'))
    expect(trigger()).not.toHaveTextContent('Grok 4.5')
  })
})

describe('AgentModelPicker · 切换代理', () => {
  beforeEach(() => {
    detectModels.mockReturnValue(new Promise(() => {}))
    detectAgents.mockResolvedValue([
      { id: 'opencode', name: 'OpenCode', available: true, models: [{ id: 'default', label: 'Default' }] },
      { id: 'claude', name: 'Claude Code', available: true, models: [{ id: 'default', label: 'Default' }] },
    ])
  })

  it('locked 时非当前项置灰且不可切换', async () => {
    const onRuntimeChange = vi.fn()
    renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'opencode' }, onRuntimeChange, conversationId: 'c1', locked: true })
    await waitFor(() => expect(detectAgents).toHaveBeenCalled())
    openPicker()
    expect(await screen.findByRole('radio', { name: /OpenCode/ })).not.toBeDisabled()
    expect(screen.getByText('已绑定')).toHaveAttribute('title', '当前会话已绑定代理')
    expect(screen.getByRole('radio', { name: 'Kivio Agent' })).toBeDisabled()
    expect(screen.getByRole('radio', { name: 'Kivio Chat' })).toBeDisabled()
    act(() => { fireEvent.click(screen.getByRole('radio', { name: 'Kivio Agent' })) })
    expect(onRuntimeChange).not.toHaveBeenCalled()
    expect(screen.getByRole('radio', { name: /Claude Code/ })).toBeDisabled()
  })

  it('当前轮运行时暂缓切换，结束后可在同一对话选择其他代理', async () => {
    const onRuntimeChange = vi.fn()
    const view = renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'opencode' }, onRuntimeChange, conversationId: 'c1', busy: true })
    openPicker()
    const claude = await screen.findByRole('radio', { name: /Claude Code/ })
    expect(claude).toBeDisabled()
    expect(screen.queryByText('已绑定')).not.toBeInTheDocument()
    detectModels.mockResolvedValue({ models: [{ id: 'claude-opus-5-5' }], source: 'probed', initialModel: 'claude-opus-5-5' })
    view.rerenderWith({ busy: false })
    act(() => { fireEvent.click(claude) })
    await waitFor(() => expect(onRuntimeChange).toHaveBeenCalledWith(expect.objectContaining({ externalAgentId: 'claude', externalModel: 'claude-opus-5-5' })))
  })

  it('ignores a first-use catalog that arrives after switching conversations', async () => {
    const onRuntimeChange = vi.fn()
    const view = renderPicker({ onRuntimeChange, conversationId: 'first' })
    let resolve!: (catalog: Partial<ExternalAgentModels>) => void
    detectModels.mockImplementation(() => new Promise(done => { resolve = done }))
    openPicker()
    fireEvent.click(await screen.findByRole('radio', { name: /Claude Code/ }))
    expect(detectModels).toHaveBeenCalledWith('claude', 'first')
    view.rerenderWith({ conversationId: 'second' })
    await act(async () => resolve({ source: 'probed', initialModel: 'claude-opus-5-5' }))
    expect(onRuntimeChange).not.toHaveBeenCalled()
  })

  it('再点当前外部代理不把模型和思考重置成 default', async () => {
    const onRuntimeChange = vi.fn()
    renderPicker({
      agentRuntime: { kind: 'external', externalAgentId: 'claude', externalModel: 'claude-opus-5', externalReasoning: 'high' },
      onRuntimeChange, conversationId: 'c1',
    })
    openPicker()
    const claude = await screen.findByRole('radio', { name: /Claude Code/ })
    act(() => { fireEvent.click(claude) })
    expect(onRuntimeChange).not.toHaveBeenCalled()
  })

  it('切到 Claude Code 时带回该代理上次的模型和思考档，弹层保持打开以便继续选模型', async () => {
    window.localStorage.setItem(LAST_AGENT_RUNTIME_KEY, JSON.stringify({
      kind: 'external',
      externalAgentId: 'dsh',
      externalModel: 'deepseek-v4-flash',
      byAgent: { claude: { externalModel: 'claude-opus-5', externalReasoning: 'high' } },
    }))
    const onRuntimeChange = vi.fn()
    renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'opencode' }, onRuntimeChange, conversationId: null })
    openPicker()
    const claude = await screen.findByRole('radio', { name: /Claude Code/ })
    act(() => { fireEvent.click(claude) })
    await waitFor(() => expect(onRuntimeChange).toHaveBeenCalledWith({
      kind: 'external',
      externalAgentId: 'claude',
      externalModel: 'claude-opus-5',
      externalReasoning: 'high',
      externalSandbox: null,
      externalAgentPreset: null,
    }))
    expect(panel()).toBeInTheDocument()
    window.localStorage.removeItem(LAST_AGENT_RUNTIME_KEY)
  })

  it('未 locked 时切换项可用', async () => {
    const onRuntimeChange = vi.fn()
    renderPicker({ agentRuntime: { ...runtime, externalAgentId: 'opencode' }, onRuntimeChange, conversationId: 'c1' })
    openPicker()
    expect(screen.queryByText('已绑定')).not.toBeInTheDocument()
    expect(screen.getByRole('radio', { name: 'Kivio Agent' })).not.toBeDisabled()
    act(() => { fireEvent.click(screen.getByRole('radio', { name: 'Kivio Agent' })) })
    expect(onRuntimeChange).toHaveBeenCalledWith(expect.objectContaining({ kind: 'builtin' }))
  })
})

describe('AgentModelPicker · built-in models and thinking level', () => {
  it('lists provider models in the panel and closes after picking one', async () => {
    const onModelChange = vi.fn()
    renderPicker({ onModelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('High'))
    expect(trigger()).toHaveTextContent('m1')
    openPicker()
    expect(within(panel()).getByRole('group', { name: 'Provider One' })).toBeInTheDocument()
    fireEvent.click(within(panel()).getByRole('button', { name: 'm2' }))
    expect(onModelChange).toHaveBeenCalledWith('p1', 'm2')
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('hides the effort control without writing a stale choice when capabilities are empty', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'unsupported' })
    const onThinkingLevelChange = vi.fn()
    await act(async () => { renderPicker({ thinkingLevel: 'high', model: 'no-levels', onThinkingLevelChange }) })
    expect(trigger().querySelector('.chat-thinking-level-label')).toBeNull()
    openPicker()
    expect(screen.queryByRole('menuitemradio')).not.toBeInTheDocument()
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
  })

  it('选择某一档回调原始等级值，且不关闭弹层', async () => {
    const onThinkingLevelChange = vi.fn()
    renderPicker({ thinkingLevel: 'high', onThinkingLevelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('High'))
    openPicker()
    await waitFor(() => expect(screen.getByRole('menuitemradio', { name: 'Off' })).toBeEnabled())
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Off' }))
    expect(onThinkingLevelChange).toHaveBeenCalledWith('off')
    expect(panel()).toBeInTheDocument()
  })

  it('能力列表加载完成前不会把 xhigh 回写成 high', async () => {
    let resolveLevels!: (levels: { levels: string[]; offMode: 'supported' }) => void
    thinkingCapabilitiesForModel.mockImplementationOnce(() => new Promise((resolve) => { resolveLevels = resolve }))
    const onThinkingLevelChange = vi.fn()
    renderPicker({ thinkingLevel: 'xhigh', model: 'pending-capabilities', onThinkingLevelChange })
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
    await act(async () => { resolveLevels({ levels: ['low', 'medium', 'high', 'xhigh'], offMode: 'supported' }) })
    expect(trigger()).toHaveTextContent('XHigh')
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
  })

  it('没有强度档位但支持开关时仍可关闭并重新开启', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'supported' })
    const onThinkingLevelChange = vi.fn()
    let view!: ReturnType<typeof renderPicker>
    await act(async () => { view = renderPicker({ thinkingLevel: 'high', model: 'toggle-only', onThinkingLevelChange }) })
    expect(trigger()).toHaveTextContent('On')
    openPicker()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Off' }))
    expect(onThinkingLevelChange).toHaveBeenCalledWith('off')
    view.rerenderWith({ thinkingLevel: 'off', model: 'toggle-only' })
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'On' }))
    expect(onThinkingLevelChange).toHaveBeenLastCalledWith('high')
  })

  it('始终思考模型保留旧 Off 的明确提示且不能再次选择 Off', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: ['low', 'high', 'max'], offMode: 'unsupported' })
    const onThinkingLevelChange = vi.fn()
    await act(async () => { renderPicker({ thinkingLevel: 'off', model: 'always-thinking', onThinkingLevelChange }) })
    expect(trigger()).toHaveTextContent(/Off.*(不可用|unavailable)/)
    openPicker()
    expect(screen.getByRole('menuitemradio', { name: 'Off' })).toBeDisabled()
    expect(screen.queryByRole('note')).not.toBeInTheDocument()
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Low' }))
    expect(onThinkingLevelChange).toHaveBeenCalledWith('low')
  })

  it('无强度档位的始终思考模型允许修正旧 Off 为 On', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: [], offMode: 'unsupported' })
    const onThinkingLevelChange = vi.fn()
    await act(async () => { renderPicker({ thinkingLevel: 'off', model: 'toggle-always-thinking', onThinkingLevelChange }) })
    openPicker()
    expect(screen.getByRole('menuitemradio', { name: 'Off' })).toBeDisabled()
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'On' }))
    expect(onThinkingLevelChange).toHaveBeenCalledWith('high')
  })

  it('换模型等待新能力时不使用旧档位回写选择', async () => {
    const onThinkingLevelChange = vi.fn()
    let resolveNext!: (caps: { levels: string[]; offMode: 'supported' }) => void
    const view = renderPicker({ thinkingLevel: 'high', model: 'old', onThinkingLevelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('High'))
    thinkingCapabilitiesForModel.mockImplementationOnce(() => new Promise(resolve => { resolveNext = resolve }))
    await act(async () => { view.rerenderWith({ thinkingLevel: 'xhigh', model: 'new' }) })
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
    openPicker()
    expect(screen.getByRole('menuitemradio', { name: 'High' })).toBeDisabled()
    await act(async () => { resolveNext({ levels: ['low', 'high', 'xhigh'], offMode: 'supported' }) })
    expect(trigger()).toHaveTextContent('XHigh')
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
  })

  it('「是否支持关闭思考」只作为 Off 的悬停提示，不常驻一段说明', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: ['low', 'high'], offMode: 'unknown' })
    await act(async () => { renderPicker({ thinkingLevel: 'high', model: 'unverified-off' }) })
    openPicker()
    expect(screen.getByRole('menuitemradio', { name: 'Off' })).toHaveAttribute('title', 'Off · 尚未确认此模型是否支持关闭思考。')
    expect(screen.queryByRole('note')).not.toBeInTheDocument()
    expect(screen.queryByText('尚未确认此模型是否支持关闭思考。')).not.toBeInTheDocument()
  })

  it('胶囊显示当前代理的图标：内置是 Kivio 标志，Chat 用染色变体', async () => {
    await act(async () => { renderPicker() })
    expect(trigger().querySelector('img.kv-agent-picker__logo')).not.toBeNull()
    expect(trigger().querySelector('.kv-agent-picker__logo--chat')).toBeNull()
    // 代理图标 / 模型图标：有具体模型时才带分隔符。
    expect(trigger().querySelector('.kv-agent-picker__crumb-sep')).toHaveTextContent('/')
    document.body.innerHTML = ''
    await act(async () => { renderPicker({ model: '' }) })
    expect(trigger().querySelector('.kv-agent-picker__crumb-sep')).toBeNull()
    document.body.innerHTML = ''
    await act(async () => { renderPicker({ model: 'no-brand-model' }) })
    expect(trigger().querySelector('.kv-agent-picker__crumb-sep')).toBeNull()
    expect(trigger()).toHaveTextContent('no-brand-model')
    document.body.innerHTML = ''
    await act(async () => { renderPicker({ agentRuntime: { ...builtin, kind: 'chat' } }) })
    expect(trigger().querySelector('img.kv-agent-picker__logo--chat')).not.toBeNull()
  })

  it('滑杆：方向键吸附到相邻可用档', async () => {
    thinkingCapabilitiesForModel.mockResolvedValue({ levels: ['low', 'high', 'max'], offMode: 'unsupported' })
    const onThinkingLevelChange = vi.fn()
    await act(async () => { renderPicker({ thinkingLevel: 'low', onThinkingLevelChange }) })
    openPicker()
    const low = screen.getByRole('menuitemradio', { name: 'Low' })
    expect(low).toHaveAttribute('aria-checked', 'true')
    fireEvent.keyDown(low, { key: 'ArrowRight' })
    expect(onThinkingLevelChange).toHaveBeenLastCalledWith('high')
  })

  it('滑杆拖动时只预览，松手才提交一次', async () => {
    const onThinkingLevelChange = vi.fn()
    await act(async () => { renderPicker({ thinkingLevel: 'low', onThinkingLevelChange }) })
    openPicker()
    const track = screen.getByRole('group', { name: '思考强度' })
    // 一行布局：档位名紧挨在轨道左边。
    expect(track.previousElementSibling).toHaveTextContent('Low')
    vi.spyOn(track, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 324, bottom: 24, width: 324, height: 24, toJSON: () => ({}),
    })
    // 4 档（Off/Low/Medium/High）等距落在 12..312：拖到最右是 High。
    fireEvent.pointerDown(track, { button: 0, clientX: 112, pointerId: 1 })
    fireEvent.pointerMove(track, { clientX: 320, pointerId: 1 })
    expect(onThinkingLevelChange).not.toHaveBeenCalled()
    expect(within(track.parentElement!).getByText('High')).toBeInTheDocument()
    fireEvent.pointerUp(track, { pointerId: 1 })
    expect(onThinkingLevelChange).toHaveBeenCalledTimes(1)
    expect(onThinkingLevelChange).toHaveBeenCalledWith('high')
  })

  it('搜索：有字时 Esc 先清空，回车选中高亮的首个结果', async () => {
    const onModelChange = vi.fn()
    renderPicker({ onModelChange })
    await waitFor(() => expect(trigger()).toHaveTextContent('High'))
    openPicker()
    const search = screen.getByRole('textbox')
    fireEvent.change(search, { target: { value: 'm2' } })
    fireEvent.keyDown(search, { key: 'Escape' })
    expect(search).toHaveValue('')
    expect(panel()).toBeInTheDocument()
    fireEvent.change(search, { target: { value: 'm2' } })
    // 中文输入法按回车上屏：WebKit 里 isComposing 已是 false，只有 keyCode 229，不能当成选中。
    fireEvent.keyDown(search, { key: 'Enter', keyCode: 229 })
    expect(onModelChange).not.toHaveBeenCalled()
    expect(panel()).toBeInTheDocument()
    fireEvent.keyDown(search, { key: 'Enter' })
    expect(onModelChange).toHaveBeenCalledWith('p1', 'm2')
  })

  it('方向键在模型行之间移动，不在每行的收藏星标上多停一步', async () => {
    renderPicker()
    await waitFor(() => expect(trigger()).toHaveTextContent('High'))
    openPicker()
    const m1 = within(panel()).getByRole('button', { name: 'm1' })
    m1.focus()
    fireEvent.keyDown(m1, { key: 'ArrowDown' })
    expect(within(panel()).getByRole('button', { name: 'm2' })).toHaveFocus()
  })

  it('Esc 关闭已展开的弹层', async () => {
    renderPicker()
    await waitFor(() => expect(trigger()).toHaveTextContent('High'))
    openPicker()
    expect(panel()).toBeInTheDocument()
    act(() => { fireEvent.keyDown(window, { key: 'Escape' }) })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })
})
