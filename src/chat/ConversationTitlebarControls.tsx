import { memo } from 'react'
import { ExternalLink } from 'lucide-react'
import { IconButton } from '../components/Button'
import { i18n, type Lang } from '../components/i18n'
import { ExternalModelSelector, RuntimePicker } from './RuntimePicker'
import { ModelSelector } from './ModelSelector'
import { PermissionPicker } from './PermissionPicker'
import { ExternalCliAccount } from './ExternalCliAccount'
import { ThinkingLevelSelector } from './ThinkingLevelSelector'
import { SidebarToggleIcon } from './SidebarToggleIcon'
import { usesNativeTitlebar } from './platform'
import { WorkspaceActionsMenu, type WorkspaceEnvironment } from './dock/WorkspaceActionsMenu'
import { ConversationHistoryBranches, type HistoryBranchActions } from './ConversationHistoryBranches'
import type { WorkspaceUsageSource } from './dock/WorkspaceUsageSection'
import type { AgentRuntimeConfig, ConversationContextState, DetectedExternalAgent, ThinkingLevel } from './types'

export interface ConversationTitlebarControlsProps {
  activeAgentRuntime: AgentRuntimeConfig
  externalAgent?: DetectedExternalAgent
  conversationId: string | null
  /** 有消息、或主窗已被弹出占住：锁死 kind / agent。 */
  runtimeLocked: boolean
  usesExternalRuntime: boolean
  usesChatRuntime: boolean
  activeProviderId: string
  activeModel: string
  thinkingLevel: ThinkingLevel | null
  approvalPolicy: string
  dockOpen: boolean
  uiLang: Lang
  onRuntimeChange: (runtime: AgentRuntimeConfig) => void | Promise<void>
  onExternalModelChange: (model: string, reasoning?: string | null, serviceTier?: string | null) => void | Promise<void>
  onModelChange: (providerId: string, model: string) => void | Promise<void>
  onThinkingLevelChange: (level: ThinkingLevel | null) => void | Promise<void>
  onApprovalPolicyChange: (policy: string) => void
  onSandboxChange: (value: string) => void | Promise<void>
  /** 当前会话有运行中的回合：锁住 CLI 沙盒档位。 */
  busy: boolean
  onOpenPopout: (conversationId: string) => void
  onToggleDock: () => void
  /** 关联项目且有工作目录的会话显示工作区面板。 */
  workdir: string
  /** 关联项目名；无项目对话不显示 Git。 */
  projectName: string | null
  onOpenGitPanel: () => void
  onOpenTasks: () => void
  historyBranches?: HistoryBranchActions | null
  /** 空白新对话：工作区面板默认收起。 */
  workspaceBlank?: boolean
  environment?: WorkspaceEnvironment | null
  usage?: WorkspaceUsageSource | null
  contextState?: ConversationContextState | null
}

/** 会话页顶栏控件。非 mac 渲染进全宽标题栏带，mac 仍留在主区 52px 顶栏。 */
export const ConversationTitlebarControls = memo(function ConversationTitlebarControls({
  activeAgentRuntime,
  externalAgent,
  conversationId,
  runtimeLocked,
  usesExternalRuntime,
  usesChatRuntime,
  activeProviderId,
  activeModel,
  thinkingLevel,
  approvalPolicy,
  dockOpen,
  uiLang,
  onRuntimeChange,
  onExternalModelChange,
  onModelChange,
  onThinkingLevelChange,
  onApprovalPolicyChange,
  onSandboxChange,
  busy,
  onOpenPopout,
  onToggleDock,
  workdir,
  projectName,
  onOpenGitPanel,
  onOpenTasks,
  historyBranches,
  workspaceBlank,
  environment,
  usage,
  contextState,
}: ConversationTitlebarControlsProps) {
  const showWorkspace = !usesChatRuntime && Boolean(projectName && workdir)
  return (
    <>
      <div className="flex min-w-0 items-center gap-1">
        <div className="shrink-0" data-tauri-drag-region="false">
          <RuntimePicker
            agentRuntime={activeAgentRuntime}
            onRuntimeChange={onRuntimeChange}
            conversationId={conversationId}
            locked={runtimeLocked}
          />
        </div>
        <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
          {usesExternalRuntime ? (
            <ExternalModelSelector
              agentRuntime={activeAgentRuntime}
              onModelChange={onExternalModelChange}
              conversationId={conversationId}
            />
          ) : (
            <ModelSelector
              currentProviderId={activeProviderId}
              currentModel={activeModel}
              onModelChange={onModelChange}
            />
          )}
        </div>
        {!usesExternalRuntime && (
          <div className="shrink-0 chat-thinking-pill-wrap" data-tauri-drag-region="false">
            <ThinkingLevelSelector
              currentProviderId={activeProviderId}
              currentModel={activeModel}
              value={thinkingLevel}
              onChange={onThinkingLevelChange}
            />
          </div>
        )}
        <div className="shrink-0" data-tauri-drag-region="false">
          <PermissionPicker
            agentRuntime={activeAgentRuntime}
            conversationId={conversationId}
            approvalPolicy={approvalPolicy}
            onApprovalPolicyChange={onApprovalPolicyChange}
            onSandboxChange={onSandboxChange}
            sandboxLocked={busy}
          />
        </div>
        {usesExternalRuntime && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <ExternalCliAccount agent={externalAgent} conversationId={conversationId} />
          </div>
        )}
      </div>
      <div className="min-w-5 flex-1" data-tauri-drag-region />
      <div className="flex min-w-0 shrink items-center justify-end gap-1">
        {conversationId && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <IconButton
              label={i18n[uiLang].chatOpenInNewWindow}
              size="sm"
              variant="ghost"
              onClick={() => onOpenPopout(conversationId)}
            >
              <ExternalLink size={15} strokeWidth={1.75} />
            </IconButton>
          </div>
        )}
        {!showWorkspace && conversationId && historyBranches && (
          <ConversationHistoryBranches conversationId={conversationId} lang={uiLang} {...historyBranches} />
        )}
        {showWorkspace && (
          <WorkspaceActionsMenu workdir={workdir} projectName={projectName} conversationId={conversationId} dockOpen={dockOpen} lang={uiLang} onOpenGitPanel={onOpenGitPanel} onOpenTasks={onOpenTasks} historyBranches={historyBranches} blank={workspaceBlank} environment={environment} usage={usage} contextState={contextState} />
        )}
        {usesNativeTitlebar && !usesChatRuntime && !dockOpen && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <IconButton
              label={i18n[uiLang].dockToggle}
              size="sm"
              variant="ghost"
              onClick={onToggleDock}
              className="group"
              aria-expanded={dockOpen}
            >
              <SidebarToggleIcon side="right" open={dockOpen} />
            </IconButton>
          </div>
        )}
      </div>
    </>
  )
})
