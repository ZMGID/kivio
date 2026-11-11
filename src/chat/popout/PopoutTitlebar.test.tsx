/**
 * @vitest-environment jsdom
 */
import { render } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LangContext } from '../../components/i18n'
import type { AgentRuntimeConfig } from '../types'
import { PopoutTitlebar } from './PopoutTitlebar'

vi.mock('../RuntimePicker', () => ({
  RuntimePicker: ({ locked }: { locked: boolean }) => <button data-testid="runtime-picker" disabled={locked} />,
  ExternalModelSelector: () => <span />,
}))
vi.mock('../ModelSelector', () => ({ ModelSelector: () => <span data-testid="model-selector" /> }))
vi.mock('../ThinkingLevelSelector', () => ({ ThinkingLevelSelector: () => <span /> }))
vi.mock('../PermissionPicker', () => ({ PermissionPicker: () => <span /> }))
vi.mock('../dock/WorkspaceActionsMenu', () => ({
  WorkspaceActionsMenu: ({ inlineAllowed }: { inlineAllowed?: boolean }) => (
    <span data-testid="workspace-panel" data-inline={String(inlineAllowed ?? true)} />
  ),
}))

const runtime: AgentRuntimeConfig = { kind: 'builtin' }

describe('PopoutTitlebar', () => {
  it.each([false, true])('keeps title placement and the empty conversation runtime lock (%s)', (locked) => {
    const { container } = render(
      <LangContext.Provider value="zh">
        <PopoutTitlebar
          conversation={{
            id: 'c1',
            runtime_locked: locked,
            revision: 1,
            title: '商品图片批量生成工作流搭建',
            provider_id: 'p',
            model: 'm',
            messages: [],
            created_at: 0,
            updated_at: 0,
          }}
          runtime={runtime}
          usesExternalRuntime={false}
          approvalPolicy="readonly_auto_sensitive_confirm"
          onRuntimeChange={() => {}}
          onModelChange={() => {}}
          onExternalModelChange={() => {}}
          onThinkingLevelChange={() => {}}
          onApprovalPolicyChange={() => {}}
          onSandboxChange={() => {}}
          busy={false}
          workdir=""
          projectName={null}
          lang="zh"
        />
      </LangContext.Provider>,
    )
    expect(container.querySelector('[data-testid="runtime-picker"]')).toHaveProperty('disabled', locked)
    const pills = container.querySelector('[data-popout-pills]')
    const title = container.querySelector('[data-popout-title]')
    expect(pills).toBeTruthy()
    expect(title?.textContent).toBe('商品图片批量生成工作流搭建')
    expect(title).toHaveAttribute('data-tauri-drag-region')
    expect(Boolean(pills && title && (pills.compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true)
    expect(container.querySelector('[data-testid="workspace-panel"]')).not.toBeNull()
  })

  it.each([null, 'Repo'])('offers the workspace popover with or without a project (project: %s)', (projectName) => {
    const { queryByTestId } = render(
      <LangContext.Provider value="zh">
        <PopoutTitlebar
          conversation={{ id: 'c1', revision: 1, title: 't', provider_id: 'p', model: 'm', messages: [], created_at: 0, updated_at: 0 }}
          runtime={runtime}
          usesExternalRuntime={false}
          approvalPolicy="readonly_auto_sensitive_confirm"
          onRuntimeChange={() => {}}
          onModelChange={() => {}}
          onExternalModelChange={() => {}}
          onThinkingLevelChange={() => {}}
          onApprovalPolicyChange={() => {}}
          onSandboxChange={() => {}}
          busy={false}
          workdir="/repo"
          projectName={projectName}
          lang="zh"
        />
      </LangContext.Provider>,
    )
    expect(queryByTestId('workspace-panel')).toHaveAttribute('data-inline', 'false')
  })
})
