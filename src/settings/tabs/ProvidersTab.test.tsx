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
 * 回归重点：
 *   1. 列表级（已添加卡片 / 可添加预设）与详情级的切换
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
  const view = render(<ProvidersTab {...props} />)
  return { props, rerender: (next: Props) => view.rerender(<ProvidersTab {...next} />) }
}

/** 列表里点开一家供应商的详情。 */
async function openDetail(name = 'OpenAI') {
  await userEvent.click(screen.getByRole('button', { name }))
}

/** 只有一家供应商时直接渲染并进入它的详情。 */
async function renderDetail(provider = makeProvider({ apiKeys: ['sk-aaa', 'sk-bbb'] })) {
  const view = renderTab({ selectedProvider: provider, settings: makeSettings({ providers: [provider] }) })
  await openDetail(provider.name)
  return view
}

describe('ProvidersTab 列表', () => {
  it('默认显示已添加的供应商卡片，不直接展开详情', () => {
    renderTab()
    const card = screen.getByRole('button', { name: 'OpenAI' })
    expect(within(card).getByText('https://api.openai.com/v1')).toBeTruthy()
    expect(within(card).getByText('1 个模型')).toBeTruthy()
    expect(screen.queryByText(t.baseUrl)).toBeNull()
  })

  it('点卡片进入详情，返回后回到列表', async () => {
    const { props } = renderTab()
    await openDetail()
    expect(props.onSelectProvider).toHaveBeenCalledWith('p1')
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
    expect(screen.getByDisplayValue('https://api.openai.com/v1')).toBeTruthy()

    await userEvent.click(screen.getByRole('button', { name: '全部供应商' }))
    expect(screen.queryByText(t.baseUrl)).toBeNull()
    expect(screen.getByRole('button', { name: 'OpenAI' })).toBeTruthy()
  })

  it('卡片上的开关、删除不会顺带打开详情', async () => {
    const { props } = renderTab()
    await userEvent.click(screen.getByRole('switch', { name: '启用 OpenAI' }))
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { enabled: false })
    await userEvent.click(screen.getByRole('button', { name: t.deleteProvider }))
    expect(props.onRequestDeleteProvider).toHaveBeenCalledWith('p1')
    expect(screen.queryByText(t.baseUrl)).toBeNull()
  })

  it('未填密钥的启用供应商在卡片上标出未授权', () => {
    const provider = makeProvider({ apiKeys: [], request: { oauth: { provider: 'kimi' } } })
    renderTab({ selectedProvider: provider, settings: makeSettings({ providers: [provider] }) })
    expect(within(screen.getByRole('button', { name: 'OpenAI' })).getByText(t.permissionMissing)).toBeTruthy()
  })

  it('供应商被删除后从详情退回列表', async () => {
    const { props, rerender } = renderTab()
    await openDetail()
    rerender({ ...props, selectedProvider: undefined, settings: makeSettings({ providers: [] }) })
    expect(screen.queryByText(t.baseUrl)).toBeNull()
    expect(screen.getByText(/还没有供应商/)).toBeTruthy()
  })

  it('预设不直接铺在列表页，只在添加流程里出现', async () => {
    renderTab()
    expect(screen.queryByRole('button', { name: /Alpha Test/ })).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    const dialog = screen.getByRole('dialog', { name: t.addProvider })
    expect(within(dialog).getByRole('button', { name: /Alpha Test/ })).toBeTruthy()
    expect(within(dialog).getByRole('button', { name: t.presetCustom })).toBeTruthy()
  })

  it('添加流程里选预设：赞助位在前，选中后添加并关闭弹窗', async () => {
    const { props } = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    const dialog = screen.getByRole('dialog')
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

  it('添加流程里选自定义配置：创建空白供应商，外壳选中后直接进入详情', async () => {
    const { props, rerender } = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    await userEvent.click(screen.getByRole('button', { name: t.presetCustom }))
    expect(props.onAddProvider).toHaveBeenCalled()
    expect(props.onAddProviderFromPreset).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()

    const created = makeProvider({ id: 'p-new', name: '', baseUrl: '', apiKeys: [], enabledModels: [] })
    rerender({
      ...props,
      selectedProvider: created,
      settings: makeSettings({ providers: [...props.settings.providers, created] }),
    })
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
    expect(screen.getByRole('textbox', { name: t.providerName })).toHaveValue('')
  })

  it('添加弹窗内可搜索预设；关闭后不添加任何东西', async () => {
    const { props } = renderTab()
    await userEvent.click(screen.getByRole('button', { name: t.addProvider }))
    const dialog = screen.getByRole('dialog')
    await userEvent.type(within(dialog).getByPlaceholderText(t.presetProvidersSearch), 'alpha')
    expect(within(dialog).getByRole('button', { name: /Alpha Test/ })).toBeTruthy()
    expect(within(dialog).queryByRole('button', { name: /Beta Test/ })).toBeNull()

    await userEvent.click(within(dialog).getByRole('button', { name: '关闭' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(props.onAddProvider).not.toHaveBeenCalled()
    expect(props.onAddProviderFromPreset).not.toHaveBeenCalled()
  })

  it('列表页搜索过滤已添加的供应商', async () => {
    const other = makeProvider({ id: 'p2', name: 'Anthropic', baseUrl: 'https://api.anthropic.com' })
    const provider = makeProvider()
    renderTab({ selectedProvider: provider, settings: makeSettings({ providers: [provider, other] }) })
    await userEvent.type(screen.getByPlaceholderText(t.presetProvidersSearch), 'anthropic')
    expect(screen.getByRole('button', { name: 'Anthropic' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'OpenAI' })).toBeNull()
  })

  it('多于一家且未搜索时才显示拖拽手柄', async () => {
    const other = makeProvider({ id: 'p2', name: 'Anthropic' })
    const { props, rerender } = renderTab()
    expect(screen.queryByRole('button', { name: '拖动调整顺序' })).toBeNull()
    rerender({ ...props, settings: makeSettings({ providers: [...props.settings.providers, other] }) })
    expect(screen.getAllByRole('button', { name: '拖动调整顺序' })).toHaveLength(2)
    await userEvent.type(screen.getByPlaceholderText(t.presetProvidersSearch), 'o')
    expect(screen.queryByRole('button', { name: '拖动调整顺序' })).toBeNull()
  })
})

describe('ProvidersTab 详情', () => {
  it('OpenCode Free enables model management without a key field', async () => {
    const provider = makeProvider({ name: 'OpenCode Free', baseUrl: 'https://opencode.ai/zen/v1', apiKeys: [] })
    const { props } = await renderDetail(provider)
    expect(screen.getByText(/免费模型，无需账号/)).toBeInTheDocument()
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(0)
    const manage = screen.getByRole('button', { name: /管理模型/ })
    expect(manage).toBeEnabled()
    await userEvent.click(manage)
    expect(props.onOpenModelPicker).toHaveBeenCalled()
  })

  it('密钥默认掩码显示，池中每条各一行', async () => {
    await renderDetail()
    // 两条密钥 → 两个 password 输入
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(2)
  })

  it('revealedKeys 只解掩指定那一条', async () => {
    renderTab({ revealedKeys: new Set(['p1-0']) })
    await openDetail()
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(1)
    expect(screen.getByDisplayValue('sk-aaa')).toBeTruthy()
  })

  it('删除密钥按 index 作用（删第 2 条时保留第 1 条）', async () => {
    const { props } = await renderDetail()
    const removes = document.querySelectorAll<HTMLButtonElement>('.kv-icon-btn[aria-label="移除"]')
    await userEvent.click(removes[1])
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { apiKeys: ['sk-aaa'], activeKeyIndex: 0 })
  })

  it('点备用 Key 的开关切到该条，不改密钥顺序', async () => {
    const { props } = await renderDetail()
    const current = screen.getByRole('switch', { name: t.apiKeyCurrent })
    const backup = screen.getByRole('switch', { name: t.apiKeyUse })
    expect(current.getAttribute('aria-checked')).toBe('true')
    expect(backup.getAttribute('aria-checked')).toBe('false')
    await userEvent.click(backup)
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { activeKeyIndex: 1 })
  })

  it('单条密钥时不显示切换开关和删除按钮', async () => {
    await renderDetail(makeProvider({ apiKeys: ['sk-only'] }))
    expect(screen.queryByRole('switch', { name: t.apiKeyCurrent })).toBeNull()
    expect(screen.queryByRole('switch', { name: t.apiKeyUse })).toBeNull()
    expect(document.querySelectorAll('.kv-icon-btn[aria-label="移除"]')).toHaveLength(0)
  })

  it('新增密钥追加空串而非覆盖', async () => {
    const { props } = await renderDetail()
    await userEvent.click(screen.getByRole('button', { name: t.addKey }))
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { apiKeys: ['sk-aaa', 'sk-bbb', ''] })
  })

  it('删除供应商走 onRequestDeleteProvider（不直接删）', async () => {
    const { props } = await renderDetail()
    await userEvent.click(screen.getByRole('button', { name: t.deleteProvider }))
    expect(props.onRequestDeleteProvider).toHaveBeenCalledWith('p1')
    expect(props.onUpdateProvider).not.toHaveBeenCalled()
  })

  it('顶栏开关启停供应商', async () => {
    const { props } = await renderDetail()
    await userEvent.click(screen.getByRole('switch', { name: '启用供应商' }))
    expect(props.onUpdateProvider).toHaveBeenCalledWith('p1', { enabled: false })
  })

  it('测试连接与模型管理是两个不同回调', async () => {
    const { props } = await renderDetail()
    await userEvent.click(screen.getByRole('button', { name: t.testConnection }))
    expect(props.onOpenModelTest).toHaveBeenCalledWith('p1')
    expect(props.onOpenModelPicker).not.toHaveBeenCalled()

    await userEvent.click(screen.getByRole('button', { name: /管理模型/ }))
    expect(props.onOpenModelPicker).toHaveBeenCalledWith('p1')
  })

  it('点击已启用模型打开抽屉，带 providerId + model', async () => {
    const { props } = await renderDetail()
    await userEvent.click(screen.getByTitle('gpt-4o'))
    expect(props.onOpenModelDrawer).toHaveBeenCalledWith({ providerId: 'p1', model: 'gpt-4o' })
  })

  it('移除模型不冒泡触发抽屉', async () => {
    const { props } = await renderDetail()
    await userEvent.click(document.querySelector<HTMLButtonElement>('.kv-enabled-model-remove')!)
    expect(props.onRemoveEnabledModel).toHaveBeenCalledWith('p1', 'gpt-4o')
    expect(props.onOpenModelDrawer).not.toHaveBeenCalled()
  })

  it('opens request settings, toggles gzip details, and returns to provider details', async () => {
    const { props, rerender } = await renderDetail()
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
    // 二级页接管整页：一级页的字段和供应商顶栏都不该还留在 DOM 里。
    expect(screen.queryByText(t.baseUrl)).toBeNull()
    expect(screen.queryByRole('button', { name: t.testConnection })).toBeNull()
    expect(screen.queryByRole('button', { name: '全部供应商' })).toBeNull()

    await userEvent.click(screen.getByRole('button', { name: /返回/ }))
    expect(screen.getByText(t.baseUrl)).toBeTruthy()
    expect(screen.queryByText(t.customHeaders)).toBeNull()
  })
})
