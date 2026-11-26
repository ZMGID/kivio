import type { ExternalModelChange } from './externalModelEffort'
import { memo } from 'react'
import { ExternalLink } from 'lucide-react'
import { IconButton } from '../components/Button'
import { i18n, type Lang } from '../components/i18n'
import { AgentModelPicker } from './AgentModelPicker'
import { PermissionPicker } from './PermissionPicker'
import { ExternalCliAccount } from './ExternalCliAccount'
import type { CliAccountConversation } from './useChatCliAccountScope'
import { SidebarToggleIcon } from './SidebarToggleIcon'
import { usesNativeTitlebar } from './platform'
import { WorkspaceActionsMenu, type WorkspaceEnvironment } from './dock/WorkspaceActionsMenu'
import { ConversationHistoryBranches, type HistoryBranchActions } from './ConversationHistoryBranches'
import type { WorkspaceUsageSource } from './dock/WorkspaceUsageSection'
import type { AgentRuntimeConfig, ConversationContextState, DetectedExternalAgent, ThinkingLevel } from './types'

export interface ConversationTitlebarControlsProps {
  readOnly?: boolean
  activeAgentRuntime: AgentRuntimeConfig
  externalAgent?: DetectedExternalAgent
  conversation: CliAccountConversation | null
  draftProjectId?: string | null
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
  onExternalModelChange: ExternalModelChange
  onModelChange: (providerId: string, model: string) => void | Promise<void>
  onThinkingLevelChange: (level: ThinkingLevel | null) => void | Promise<void>
  onApprovalPolicyChange: (policy: string) => void
  onSandboxChange: (value: string) => void | Promise<void>
  /** 当前会话有运行中的回合：锁住 CLI 沙盒档位。 */
  busy: boolean
  onOpenPopout: (conversationId: string) => void
  onToggleDock: () => void
  /** 会话工作目录；无项目对话也可能有（Git 仍只给项目对话）。 */
  workdir: string
  /** 关联项目名；无项目对话不显示 Git。 */
  projectName: string | null
  onOpenGitPanel: () => void
  onOpenTasks: () => void
  historyBranches?: HistoryBranchActions | null
  environment?: WorkspaceEnvironment | null
  usage?: WorkspaceUsageSource | null
  contextState?: ConversationContextState | null
}

/** 会话页顶栏控件。非 mac 渲染进全宽标题栏带，mac 仍留在主区 52px 顶栏。 */
export const ConversationTitlebarControls = memo(function ConversationTitlebarControls({
  readOnly = false,
  activeAgentRuntime,
  externalAgent,
  conversation,
  draftProjectId,
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
  environment,
  usage,
  contextState,
}: ConversationTitlebarControlsProps) {
  // 工作区面板对所有 agent 对话可用；无项目对话里不含 Git。
  const conversationId = conversation?.id ?? null
  const showWorkspace = !usesChatRuntime
  return (
    <fieldset className="contents" disabled={readOnly}>
      <div key={conversationId} className="flex min-w-0 items-center gap-1">
        <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
          <AgentModelPicker
            agentRuntime={activeAgentRuntime}
            conversationId={conversationId}
            locked={runtimeLocked}
            busy={busy}
            readOnly={readOnly}
            providerId={activeProviderId}
            model={activeModel}
            thinkingLevel={thinkingLevel}
            onRuntimeChange={onRuntimeChange}
            onModelChange={onModelChange}
            onExternalModelChange={onExternalModelChange}
            onThinkingLevelChange={onThinkingLevelChange}
          />
        </div>
        <div className="shrink-0" data-tauri-drag-region="false">
          <PermissionPicker
            agentRuntime={activeAgentRuntime}
            externalAgent={externalAgent}
            approvalPolicy={approvalPolicy}
            onApprovalPolicyChange={onApprovalPolicyChange}
            onSandboxChange={onSandboxChange}
            sandboxLocked={busy}
          />
        </div>
        {usesExternalRuntime && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <ExternalCliAccount agent={externalAgent} conversation={conversation} draftProjectId={draftProjectId} />
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
          <WorkspaceActionsMenu workdir={workdir} projectName={projectName} conversationId={conversationId} dockOpen={dockOpen} lang={uiLang} readOnly={readOnly} onOpenGitPanel={onOpenGitPanel} onOpenTasks={onOpenTasks} historyBranches={historyBranches} environment={environment} usage={usage} contextState={contextState} />
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
    </fieldset>
  )
})
