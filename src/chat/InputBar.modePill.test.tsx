vi.mock('./ComposerEditor', () => import('./ComposerEditor.testSupport'))
import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'
import { deriveDshPresetModes, derivePermissionModes } from './permissionModes'
import type { AgentRuntimeConfig } from './types'
import { useState } from 'react'
import { setGoalDraftMode, useGoalDraft } from './goalPresentation'
import { setComposerDraft } from './composerDraft'
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

  it('selects Goal via Shift+Tab and leaves it without losing the objective', async () => {
    const key = 'goal-keyboard'
    setComposerDraft(key, { input: '检查商品', quotes: [], attachments: [] })
    render(<GoalDraftComposer conversationId={key} />)
    await act(async () => { fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Tab', shiftKey: true }) })
    expect(screen.getByRole('textbox')).toHaveValue('/goal 检查商品')
    await act(async () => { fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Tab', shiftKey: true }) })
    expect(screen.getByRole('textbox')).toHaveValue('检查商品')
  })

  it('Goal occupies the composer status row and replaces its Todo indicator', () => {
    render(<InputBar onSend={() => {}} goalSlot={<div data-testid="goal">Goal status</div>}
      agentTodoState={{ items: [{ id: 'todo', content: 'pending work', status: 'pending' }] }} />)
    expect(screen.getByTestId('goal').closest('.chat-composer-status')).toBeInTheDocument()
    expect(screen.queryByText('Todo')).not.toBeInTheDocument()
  })

  it('内置模型会话不显示模式胶囊，模式靠 Shift+Tab 和斜杠命令切换', () => {
    renderComposer(derivePermissionModes({
      target: 'composer',
      agentRuntime: { kind: 'builtin' },
      agentPlanMode: 'act',
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
    const presets = deriveDshPresetModes(runtime)
    render(
      <InputBar
        onSend={() => {}}
        presetOptions={presets.options}
        presetValue={presets.current}
        onPresetChange={onPresetChange}
      />,
    )
    expect(screen.getByTitle('切换 Agent 模式')).toHaveTextContent('标准模式')

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
