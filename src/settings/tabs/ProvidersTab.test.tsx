import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ProvidersTab } from './ProvidersTab'
import { makeSettings, makeProvider } from './testFixtures'
import { i18n } from '../../components/i18n'

const t = i18n.zh

vi.mock('../../api/tauri', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../api/tauri')
  return { ...actual, api: { openExternal: vi.fn() } }
})

vi.mock('../providerPresets', () => ({
  PROVIDER_PRESETS: [
    { name: 'Beta Test', baseUrl: 'https://beta.example/v1' },
    { name: 'Alpha Test', baseUrl: 'https://alpha.example/v1', sponsored: true },
    { name: 'Gamma Test', baseUrl: 'https://gamma.example/v1' },
  ],
}))

type Props = Parameters<typeof ProvidersTab>[0]

/**
 * 回归重点（拆成 ProvidersTab + ProviderDetail 两个文件后尤其要验）：
 *   1. 选中/未选中供应商的两种形态
 *   2. 密钥池的显隐、增删按 index 作用到正确的那一条
 *   3. 各回调不串（更新 / 删除 / 模型抽屉 / 测试连接）
 */
function buildProps(overrides: Partial<Props> = {}): Props {
  const provider = makeProvider({ apiKeys: ['sk-aaa', 'sk-bbb'] })
  const props: Props = {
    settings: makeSettings({ providers: [provider] }),
    t,
    lang: 'zh',
    selectedProvider: provider,
    revealedKeys: new Set<string>(),
    gzipInfoOpen: new Set<string>(),
    onSelectProvider: vi.fn(),
    onReorderProviders: vi.fn(),
    onAddProvider: vi.fn(),
    onAddProviderFromPreset: vi.fn(),
    onUpdateProvider: vi.fn(),
    onSetProviderIcon: vi.fn(),
    onRequestDeleteProvider: vi.fn(),
    onToggleGzipInfo: vi.fn(),
    onToggleKeyReveal: vi.fn(),
    onOpenModelPicker: vi.fn(),
    onOpenModelTest: vi.fn(),
    onOpenModelDrawer: vi.fn(),
    onRemoveEnabledModel: vi.fn(),
    ...overrides,
  }
  return props
}

function renderTab(overrides: Partial<Props> = {}) {
  const props = buildProps(overrides)
  render(<ProvidersTab {...props} />)
  return props
}

/** 需要在同一棵树上换 props 的用例（如切换供应商）用它，别重新 render 出第二棵树。 */
function renderTabWithRerender(overrides: Partial<Props> = {}) {
  const props = { ...buildProps(overrides) }
  const view = render(<ProvidersTab {...props} />)
  return { props, rerender: (next: Props) => view.rerender(<ProvidersTab {...next} />) }
}

describe('ProvidersTab', () => {
  it('OpenCode Free enables model management without a key field', async () => {
    const provider = makeProvider({ name: 'OpenCode Free', baseUrl: 'https://opencode.ai/zen/v1', apiKeys: [] })
    const props = renderTab({ selectedProvider: provider, settings: makeSettings({ providers: [provider] }) })
    expect(screen.getByText(/免费模型，无需账号/)).toBeInTheDocument()
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(0)
    const manage = screen.getByRole('button', { name: /管理模型/ })
    expect(manage).toBeEnabled()
    await userEvent.click(manage)
    expect(props.onOpenModelPicker).toHaveBeenCalled()
  })

  it('切换供应商选择时显示对应详情', () => {
    const { props, rerender } = renderTabWithRerender({ selectedProvider: undefined })
    expect(screen.getByText(/在左侧选择供应商/)).toBeTruthy()
    expect(screen.queryByText(t.baseUrl)).toBeNull()
    rerender({ ...props, selectedProvider: props.settings.providers[0] })
    expect(screen.queryByText(/在左侧选择供应商/)).toBeNull()
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
    expect(screen.getByDisplayValue(props.settings.providers[0].baseUrl)).toBeTruthy()
  })

  it('密钥默认掩码显示，池中每条各一行', () => {
    renderTab()
    // 两条密钥 → 两个 password 输入
    const masked = document.querySelectorAll('input[type="password"]')
    expect(masked).toHaveLength(2)
  })

  it('revealedKeys 只解掩指定那一条', () => {
    renderTab({ revealedKeys: new Set(['p1-0']) })
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(1)
    expect(screen.getByDisplayValue('sk-aaa')).toBeTruthy()
  })

  it('删除密钥按 index 作用（删第 2 条时保留第 1 条）', async () => {
    const props = renderTab()
    const removes = document.querySelectorAll<HTMLButtonElement>('.kv-icon-btn[aria-label="移除"]')
    await userEvent.click(removes[1])
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { apiKeys: ['sk-aaa'], activeKeyIndex: 0 })
  })

  it('点备用 Key 的开关切到该条，不改密钥顺序', async () => {
    const props = renderTab()
    const current = screen.getByRole('switch', { name: t.apiKeyCurrent })
    const backup = screen.getByRole('switch', { name: t.apiKeyUse })
    expect(current.getAttribute('aria-checked')).toBe('true')
    expect(backup.getAttribute('aria-checked')).toBe('false')
    await userEvent.click(backup)
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { activeKeyIndex: 1 })
  })

  it('单条密钥时不显示切换开关', () => {
    renderTab({ selectedProvider: makeProvider({ apiKeys: ['sk-only'] }) })
    expect(screen.queryByRole('switch', { name: t.apiKeyCurrent })).toBeNull()
    expect(screen.queryByRole('switch', { name: t.apiKeyUse })).toBeNull()
  })

  it('新增密钥追加空串而非覆盖', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addKey }))
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { apiKeys: ['sk-aaa', 'sk-bbb', ''] })
  })

  it('单条密钥时不显示删除按钮', () => {
    renderTab({ selectedProvider: makeProvider({ apiKeys: ['sk-only'] }) })
    expect(document.querySelectorAll('.kv-icon-btn[aria-label="移除"]')).toHaveLength(0)
  })

  it('删除供应商走 onRequestDeleteProvider（不直接删）', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.deleteProvider }))
    expect(props.onRequestDeleteProvider).toHaveBeenCalledWith('p1')
    expect(props.onUpdateProvider).not.toHaveBeenCalled()
  })


  it('测试连接与模型管理是两个不同回调', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.testConnection }))
    expect(props.onOpenModelTest).toHaveBeenCalledWith('p1')
    expect(props.onOpenModelPicker).not.toHaveBeenCalled()

    await userEvent.click(screen.getByRole('button', { name: /管理模型/ }))
    expect(props.onOpenModelPicker).toHaveBeenCalledWith('p1')
  })

  it('点击已启用模型打开抽屉，带 providerId + model', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByTitle('gpt-4o'))
    expect(props.onOpenModelDrawer).toHaveBeenCalledWith({ providerId: 'p1', model: 'gpt-4o' })
  })

  it('移除模型不冒泡触发抽屉', async () => {
    const props = renderTab()
    await userEvent.click(document.querySelector<HTMLButtonElement>('.kv-enabled-model-remove')!)
    expect(props.onRemoveEnabledModel).toHaveBeenCalledWith('p1', 'gpt-4o')
    expect(props.onOpenModelDrawer).not.toHaveBeenCalled()
  })

  it('opens presets and adds the selected provider, with sponsored entries first', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    const dialog = screen.getByRole('dialog', { name: t.presetProviders })
    expect(props.onAddProvider).not.toHaveBeenCalled()
    const sponsored = within(dialog).getByRole('button', { name: /Alpha Test/ })
    const regular = within(dialog).getByRole('button', { name: /Beta Test/ })
    expect(sponsored.compareDocumentPosition(regular) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    await userEvent.click(sponsored)
    expect(props.onAddProviderFromPreset).toHaveBeenCalledWith({
      name: 'Alpha Test', baseUrl: 'https://alpha.example/v1', sponsored: true,
    })
    expect(screen.queryByRole('dialog')).toBeNull()
  })


  it('选择自定义配置时创建空白供应商', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    await userEvent.click(screen.getByRole('button', { name: t.presetCustom }))
    expect(props.onAddProvider).toHaveBeenCalled()
    expect(props.onAddProviderFromPreset).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('预设网格支持搜索过滤', async () => {
    renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    const dialog = screen.getByRole('dialog', { name: t.presetProviders })
    await userEvent.type(screen.getByPlaceholderText(t.presetProvidersSearch), 'alpha')
    expect(within(dialog).getByRole('button', { name: /Alpha Test/ })).toBeTruthy()
    expect(within(dialog).queryByRole('button', { name: /Beta Test/ })).toBeNull()
  })

  it('预设弹层点关闭后详情仍在', async () => {
    renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    expect(screen.getByRole('dialog')).toBeTruthy()
    await userEvent.click(screen.getByRole('button', { name: '关闭' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
  })

  it('opens request settings, toggles gzip details, and returns to provider details', async () => {
    const { props, rerender } = renderTabWithRerender()
    expect(screen.queryByText(t.customHeaders)).toBeNull()
    expect(screen.queryByRole('switch', { name: '压缩请求体 (gzip)' })).toBeNull()

    await userEvent.click(screen.getByRole('button', { name: new RegExp(t.requestConfig) }))
    expect(screen.getByText(t.customHeaders)).toBeTruthy()
    expect(screen.getByRole('switch', { name: '压缩请求体 (gzip)' })).toBeInTheDocument()
    expect(screen.queryByText(/WAF 会扫描明文请求体/)).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: '显示说明' }))
    expect(props.onToggleGzipInfo).toHaveBeenCalledWith('p1')
    rerender({ ...props, gzipInfoOpen: new Set(['p1']) })
    expect(screen.getByText(/WAF 会扫描明文请求体/)).toBeTruthy()
    // 二级页接管右栏：一级页的字段不该还留在 DOM 里。
    expect(screen.queryByText(t.baseUrl)).toBeNull()

    await userEvent.click(screen.getByRole('button', { name: /返回/ }))
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
    expect(screen.queryByText(t.customHeaders)).toBeNull()
  })

  it('切换供应商时退回一级页，避免改到别人的请求头', async () => {
    const other = makeProvider({ id: 'p2', name: 'Anthropic' })
    const { rerender, props } = renderTabWithRerender()
    await userEvent.click(screen.getByRole('button', { name: new RegExp(t.requestConfig) }))
    expect(screen.getByText(t.customHeaders)).toBeTruthy()

    rerender({ ...props, selectedProvider: other })
    expect(screen.queryByText(t.customHeaders)).toBeNull()
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
  })
})
