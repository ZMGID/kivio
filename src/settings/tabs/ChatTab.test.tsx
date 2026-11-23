import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi, type Mock } from 'vitest'
import { ChatTab } from './ChatTab'

vi.mock('@tauri-apps/api/path', () => ({
  homeDir: async () => '/home/test',
  join: async (...parts: string[]) => parts.join('/'),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
import { makeSettings, makeProvider } from './testFixtures'
import { i18n } from '../../components/i18n'

const t = i18n.zh

type Props = Parameters<typeof ChatTab>[0]
/** 回调字段替换成 vi.fn Mock，测试里才能用 mockClear / mock.calls。 */
type MockedProps = Omit<Props, 'onUpdateChat' | 'onUpdateVoice' | 'onUpdateNativeTools'> & {
  onUpdateChat: ReturnType<typeof vi.fn>
  onUpdateVoice: Mock
  onUpdateNativeTools: ReturnType<typeof vi.fn>
}

/**
 * 回归重点：
 *   1. onUpdateChat vs onUpdateNativeTools 分流
 *   2. 最大输出 token 的「生效值 / 来源标签 / 模型名」不串位
 *   3. PromptField：空值显示 defaultText；恢复默认写回 ''
 */
function renderTab(overrides: Partial<MockedProps> = {}) {
  const props: MockedProps = {
    settings: makeSettings({ providers: [makeProvider()] }),
    t,
    lang: 'zh',
    chatConfig: {
      streamEnabled: true,
      thinkingEnabled: true,
      maxOutputTokens: 8192,
      defaultLanguage: '',
      userDisplayName: '小明',
      userAvatar: '',
      systemPrompt: '',
    } as Props['chatConfig'],
    chatTools: { enabled: false, servers: [], nativeTools: { workingDirectory: '/w' } } as unknown as Props['chatTools'],
    chatDefaults: 'You are the AI assistant inside Kivio.',
    effectiveChatMaxOutput: { maxOutput: 131072, source: 'override' },
    chatMaxOutputSourceLabel: '来自模型覆盖',
    chatMaxOutputModelLabel: 'p1 / gpt-4o',
    onUpdateChat: vi.fn(),
    onUpdateVoice: vi.fn(),
    onUpdateNativeTools: vi.fn(),
    ...overrides,
  }
  render(<ChatTab {...props} />)
  return props
}

function getAgentPromptRestoreButton() {
  const row = screen.getByText(t.kivioChatAgentSystemPrompt).closest<HTMLElement>('.kv-row')
  if (!row) throw new Error('Agent 系统提示词设置行未找到')
  return within(row).getByRole('button', { name: t.restoreDefaultPrompt })
}

describe('ChatTab', () => {
  it('隐藏工具与扩展和 Kivio Chat 设置，保留 Agent 系统提示词', () => {
    renderTab()
    expect(screen.getByText(t.kivioChatAgentSystemPrompt)).toBeInTheDocument()
    expect(screen.queryByText(t.chatToolsSection)).not.toBeInTheDocument()
    expect(screen.queryByRole('switch', { name: t.toolsOnDemand })).not.toBeInTheDocument()
    expect(screen.queryByText(t.kivioChatSection)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: `${t.promptEdit} ${t.kivioChatSystemPrompt}` })).not.toBeInTheDocument()
  })


  it('工作目录输入走 onUpdateNativeTools 而非 onUpdateChat', async () => {
    const props = renderTab()
    await userEvent.type(screen.getByDisplayValue('/w'), 'x')
    expect(props.onUpdateNativeTools).toHaveBeenCalled()
    expect(props.onUpdateChat).not.toHaveBeenCalled()
  })

  it('用户名输入走 onUpdateChat', async () => {
    const props = renderTab()
    await userEvent.type(screen.getByDisplayValue('小明'), 'x')
    expect(props.onUpdateChat).toHaveBeenCalled()
    expect(props.onUpdateNativeTools).not.toHaveBeenCalled()
  })

  it('最大输出 token 显示生效值 + 来源标签 + 模型名（三者不串）', () => {
    renderTab()
    expect(screen.getByText('131,072 tokens')).toBeTruthy()
    expect(screen.getByText('来自模型覆盖')).toBeTruthy()
    expect(screen.getByText(/p1 \/ gpt-4o/)).toBeTruthy()
    // 兜底值并入同一行说明
    expect(screen.getByText(/兜底 16,384 tokens/)).toBeTruthy()
  })

  it('source=fallback 时生效值与兜底值一致', () => {
    renderTab({
      effectiveChatMaxOutput: { maxOutput: 16384, source: 'fallback' },
      chatMaxOutputSourceLabel: '兜底设置',
    })
    expect(screen.getByText('16,384 tokens')).toBeTruthy()
    expect(screen.getByText(/兜底 16,384 tokens/)).toBeTruthy()
    expect(screen.getByText('兜底设置')).toBeTruthy()
  })

  it('Agent 提示词不在设置页直接展开，空值时弹窗以内置 defaultText 为起点', async () => {
    const props = renderTab()
    expect(screen.queryByDisplayValue('You are the AI assistant inside Kivio.')).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.kivioChatAgentSystemPrompt}` }))
    const dialog = screen.getByRole('dialog', { name: t.kivioChatAgentSystemPrompt })
    expect(within(dialog).getByRole('textbox')).toHaveValue('You are the AI assistant inside Kivio.')
    // 未改动直接保存不写入，继续跟随内置默认
    await userEvent.click(within(dialog).getByRole('button', { name: t.save }))
    expect(props.onUpdateChat).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('恢复默认写回空字符串', async () => {
    const props = renderTab({
      chatConfig: {
        streamEnabled: true,
        thinkingEnabled: true,
        maxOutputTokens: 8192,
        defaultLanguage: '',
        userDisplayName: '小明',
        userAvatar: '',
        systemPrompt: '自定义 Agent 提示',
      } as Props['chatConfig'],
    })
    await userEvent.click(getAgentPromptRestoreButton())
    expect(props.onUpdateChat).toHaveBeenCalledWith({ systemPrompt: '' })
  })

  it('无默认提示词时 Agent 恢复默认按钮禁用', () => {
    renderTab({ chatDefaults: undefined })
    expect(getAgentPromptRestoreButton()).toBeDisabled()
  })

  it('编辑 Agent 提示词写回 onUpdateChat', async () => {
    const props = renderTab({
      chatConfig: {
        streamEnabled: true,
        thinkingEnabled: true,
        maxOutputTokens: 8192,
        defaultLanguage: '',
        userDisplayName: '小明',
        userAvatar: '',
        systemPrompt: '自定义 Agent 提示',
      } as Props['chatConfig'],
    })
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.kivioChatAgentSystemPrompt}` }))
    const dialog = screen.getByRole('dialog')
    await userEvent.type(within(dialog).getByDisplayValue('自定义 Agent 提示'), 'z')
    expect(props.onUpdateChat).not.toHaveBeenCalled()
    await userEvent.click(within(dialog).getByRole('button', { name: t.save }))
    expect(props.onUpdateChat).toHaveBeenCalledWith({ systemPrompt: '自定义 Agent 提示z' })
  })

  it('取消编辑不写回', async () => {
    const props = renderTab()
    await userEvent.click(screen.getByRole('button', { name: `${t.promptEdit} ${t.kivioChatAgentSystemPrompt}` }))
    const dialog = screen.getByRole('dialog')
    await userEvent.type(within(dialog).getByRole('textbox'), 'z')
    await userEvent.click(within(dialog).getByText(t.cancel))
    expect(props.onUpdateChat).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})
