vi.mock('./ComposerEditor', () => import('./ComposerEditor.testSupport'))
import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'
import { deriveDshPresetModes, derivePermissionModes } from './permissionModes'
import type { AgentRuntimeConfig, DetectedExternalAgent } from './types'
import { useState } from 'react'
import { setGoalDraftMode, useGoalDraft } from './goalPresentation'
import { getComposerDraft, setComposerDraft } from './composerDraft'
import type { AgentPlanMode } from './types'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }),
}))
vi.mock('../api/tauri', () => ({ api: {}, isTauriRuntime: () => false }))
vi.mock('./api', () => ({
  chatApi: {
    getProjects: () => Promise.resolve([]),
    listExternalCliSlashCommands: () => Promise.resolve({ commands: [] }),
  },
}))

const claudeAgents: DetectedExternalAgent[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    available: true,
    models: [],
    sandboxOptions: [
      { id: 'plan', label: '计划 (只读)' },
      { id: 'default', label: '每次确认' },
      { id: 'acceptEdits', label: '接受编辑' },
      { id: 'bypassPermissions', label: '完全 (默认)' },
    ],
  },
]

function renderComposer(modes: { options: ReturnType<typeof derivePermissionModes>['options']; current: string }, onModeChange = vi.fn()) {
  render(
    <InputBar
      onSend={() => {}}
      modeOptions={modes.options}
      modeValue={modes.current}
      onModeChange={onModeChange}
    />,
  )
  return onModeChange
}

function GoalDraftComposer({ conversationId }: { conversationId: string }) {
  const goalDraft = useGoalDraft(conversationId)
  const [mode, setMode] = useState<AgentPlanMode>('act')
  const modes = derivePermissionModes({
    target: 'composer', agentRuntime: { kind: 'builtin' }, agentPlanMode: mode, goalActive: goalDraft,
  })
  return <InputBar conversationId={conversationId} onSend={() => {}}
    modeOptions={modes.options} modeValue={modes.current}
    onModeChange={value => {
      setGoalDraftMode(conversationId, value === 'goal')
      if (value !== 'goal') setMode(value as AgentPlanMode)
    }} />
}
function openModeMenu(pillLabel: string) {
  act(() => {
    fireEvent.click(screen.getByTitle('切换模式'))
  })
  expect(screen.getByTitle('切换模式')).toHaveTextContent(pillLabel)
}

describe('InputBar 底栏模式胶囊', () => {
  it('shows Git only for project chats and hides it when leaving the project', () => {
    const props = { onSend: () => {}, gitWorkdir: '/shared', gitLang: 'en' as const, onOpenGitPanel: () => {} }
    const view = render(<InputBar {...props} />)
    expect(screen.queryByTitle('Git')).not.toBeInTheDocument()
    view.rerender(<InputBar {...props} conversationProject={{ id: 'project', name: 'Project' }} />)
    const git = screen.getByTitle('Git')
    expect(git.closest('.chat-composer-status')).toBeInTheDocument()
    expect(git.closest('.chat-composer-tools')).not.toBeInTheDocument()
    view.rerender(<InputBar {...props} conversationProject={null} />)
    expect(screen.queryByTitle('Git')).not.toBeInTheDocument()
  })

  it('keeps project and Git as separate status actions and opens the project picker', async () => {
    const selectProject = vi.fn()
    const project = { id: 'project', name: 'A very long project name', created_at: 0, updated_at: 0 }
    render(<InputBar onSend={() => {}} selectedProject={project} showProjectEntry onSelectProject={selectProject}
      gitWorkdir="/shared" gitLang="en" onOpenGitPanel={() => {}} contextSlot="Context" />)

    const projectButton = screen.getByTitle(`项目 · ${project.name}`)
    const gitButton = screen.getByTitle('Git')
    expect(projectButton.closest('.chat-composer-environment')).toBe(gitButton.closest('.chat-composer-environment'))
    expect(projectButton).not.toContainElement(gitButton)
    expect(projectButton.querySelector('.lucide-chevron-down')).toBeInTheDocument()
    expect(screen.getByText('Context').closest('.chat-composer-tools')).toBeInTheDocument()

    fireEvent.click(gitButton)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(projectButton).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(gitButton)
    await act(async () => { fireEvent.click(projectButton) })
    expect(projectButton).toHaveAttribute('aria-expanded', 'true')
    await act(async () => { fireEvent.click(screen.getByText('退出项目工作')) })
    expect(selectProject).toHaveBeenCalledWith(null)
  })

  it('keeps collection exit available without showing Git', () => {
    const selectSet = vi.fn()
    const set = { id: 'set', name: 'Collection', created_at: 0, updated_at: 0 }
    render(<InputBar onSend={() => {}} selectedSet={set} onSelectSet={selectSet}
      gitWorkdir="/shared" gitLang="en" onOpenGitPanel={() => {}} />)
    fireEvent.click(screen.getByTitle('集 · Collection（点击退出）'))
    expect(selectSet).toHaveBeenCalledWith(null)
    expect(screen.queryByTitle('Git')).not.toBeInTheDocument()
  })

  it.each(['mouse', 'keyboard'])('selects Goal once via %s and leaves it without losing the objective', async (method) => {
    const key = `goal-${method}`
    setComposerDraft(key, { input: '检查商品', quotes: [], attachments: [] })
    render(<GoalDraftComposer conversationId={key} />)
    if (method === 'mouse') {
      openModeMenu('Act')
      await act(async () => { fireEvent.click(screen.getByRole('menuitemradio', { name: /Goal/ })) })
    } else {
      await act(async () => { fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Tab', shiftKey: true }) })
    }
    expect(screen.getByTitle('切换模式')).toHaveTextContent('Goal')
    expect(screen.getByRole('textbox')).toHaveValue('/goal 检查商品')
    openModeMenu('Goal')
    await act(async () => { fireEvent.click(screen.getByRole('menuitemradio', { name: /Goal/ })) })
    expect(getComposerDraft(key)?.input).toBe('/goal 检查商品')
    await act(async () => { fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Tab', shiftKey: true }) })
    expect(screen.getByTitle('切换模式')).toHaveTextContent('Plan')
    expect(screen.getByRole('textbox')).toHaveValue('检查商品')
  })

  it('Goal occupies the composer status row and replaces its Todo indicator', () => {
    render(<InputBar onSend={() => {}} goalSlot={<div data-testid="goal">Goal status</div>}
      agentTodoState={{ items: [{ id: 'todo', content: 'pending work', status: 'pending' }] }} />)
    expect(screen.getByTestId('goal').closest('.chat-composer-status')).toBeInTheDocument()
    expect(screen.queryByText('Todo')).not.toBeInTheDocument()
  })

  it('内置模型会话显示 Act / Goal / Plan / Orchestrate 四档', () => {
    const runtime: AgentRuntimeConfig = { kind: 'builtin' }
    renderComposer(derivePermissionModes({
      target: 'composer',
      agentRuntime: runtime,
      agentPlanMode: 'act',
    }))
    openModeMenu('Act')
    const items = screen.getAllByRole('menuitemradio')
    expect(items).toHaveLength(4)
    for (const name of ['Act', 'Goal', 'Plan', 'Orchestrate']) {
      expect(screen.getByRole('menuitemradio', { name: new RegExp(`^${name}`) })).toBeInTheDocument()
    }
    expect(screen.getByRole('menuitemradio', { name: /^Act/ })).toHaveAttribute('aria-checked', 'true')
  })


  it('本地 CLI 会话显示该 CLI 的档位，点选回传档位 id', () => {
    const runtime: AgentRuntimeConfig = {
      kind: 'external',
      externalAgentId: 'claude',
      externalSandbox: 'plan',
    }
    const onModeChange = renderComposer(derivePermissionModes({
      target: 'composer',
      agentRuntime: runtime,
      agents: claudeAgents,
    }))
    openModeMenu('计划 (只读)')
    const items = screen.getAllByRole('menuitemradio')
    expect(items.map((item) => item.textContent)).toEqual(claudeAgents[0]!.sandboxOptions!.map((option) => option.label))
    expect(items[0]).toHaveAttribute('aria-checked', 'true')

    act(() => {
      fireEvent.click(items[2])
    })
    expect(onModeChange).toHaveBeenCalledWith('acceptEdits')
  })

  it('renders the OpenCode plan capsule and sends the selected mode', () => {
    const runtime: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'opencode', externalSandbox: 'plan' }
    const agents: DetectedExternalAgent[] = [{
      id: 'opencode', name: 'OpenCode', available: true, models: [],
      sandboxOptions: [{ id: 'build', label: '构建 (默认)' }, { id: 'plan', label: '计划 (只读)' }],
    }]
    const onModeChange = renderComposer(derivePermissionModes({ target: 'composer', agentRuntime: runtime, agents }))
    openModeMenu('计划 (只读)')
    const plan = screen.getByRole('menuitemradio', { name: /计划 \(只读\)/ })
    expect(plan).toHaveAttribute('aria-checked', 'true')
    act(() => { fireEvent.click(screen.getByRole('menuitemradio', { name: /构建 \(默认\)/ })) })
    expect(onModeChange).toHaveBeenCalledWith('build')
  })

  it('该 CLI 没有档位时胶囊整个不渲染', () => {
    const runtime: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'opencode' }
    renderComposer(derivePermissionModes({
      target: 'composer',
      agentRuntime: runtime,
      agents: [{ id: 'opencode', name: 'OpenCode', available: true, models: [], sandboxOptions: [] }],
    }))
    expect(screen.queryByTitle('切换模式')).not.toBeInTheDocument()
  })

  it('dsh 会话在权限胶囊左边显示 Agent 模式胶囊', () => {
    const runtime: AgentRuntimeConfig = {
      kind: 'external',
      externalAgentId: 'dsh',
      externalSandbox: 'workspace-write',
      externalAgentPreset: 'standard',
    }
    const onPresetChange = vi.fn()
    const agents: DetectedExternalAgent[] = [{
      id: 'dsh',
      name: 'DeepSeek Harness',
      available: true,
      models: [],
      sandboxOptions: [
        { id: 'read-only', label: '只读' },
        { id: 'workspace-write', label: '工作区写 (默认)' },
        { id: 'danger-full-access', label: '完全' },
      ],
    }]
    const modes = derivePermissionModes({ target: 'composer', agentRuntime: runtime, agents })
    const presets = deriveDshPresetModes(runtime)
    render(
      <InputBar
        onSend={() => {}}
        modeOptions={modes.options}
        modeValue={modes.current}
        onModeChange={vi.fn()}
        presetOptions={presets.options}
        presetValue={presets.current}
        onPresetChange={onPresetChange}
      />,
    )
    expect(screen.getByTitle('切换 Agent 模式')).toHaveTextContent('标准模式')
    expect(screen.getByTitle('切换模式')).toHaveTextContent('工作区写 (默认)')

    act(() => {
      fireEvent.click(screen.getByTitle('切换 Agent 模式'))
    })
    const custom = deriveDshPresetModes(runtime, [
      { id: 'code-review', label: '代码审查', description: '只读评审' },
    ])
    expect(custom.options.map((option) => option.value)).toContain('code-review')
    act(() => {
      fireEvent.click(screen.getByRole('menuitemradio', { name: /^极简模式/ }))
    })
    expect(onPresetChange).toHaveBeenCalledWith('minimal')
  })

  it('已有对话时 Agent 模式菜单不可改档', () => {
    const runtime: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'dsh', externalAgentPreset: 'code' }
    const onPresetChange = vi.fn()
    const presets = deriveDshPresetModes(runtime)
    render(
      <InputBar
        onSend={() => {}}
        presetOptions={presets.options}
        presetValue={presets.current}
        onPresetChange={onPresetChange}
        presetLocked
        presetLockedReason="已有对话内容后不能切换 Agent 模式"
      />,
    )
    act(() => {
      fireEvent.click(screen.getByTitle('已有对话内容后不能切换 Agent 模式'))
    })
    const items = screen.getAllByRole('menuitemradio')
    expect(items[0]).toBeDisabled()
    act(() => {
      fireEvent.click(items[0])
    })
    expect(onPresetChange).not.toHaveBeenCalled()
  })
})
