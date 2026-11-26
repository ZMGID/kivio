import type { ExternalModelChange } from '../externalModelEffort'
import { ChatTitlebar } from '../ChatTitlebar'
import { PermissionPicker } from '../PermissionPicker'
import { ExternalCliAccount } from '../ExternalCliAccount'
import { AgentModelPicker } from '../AgentModelPicker'
import { conversationTitleSource, displayConversationTitle } from '../conversationTitle'
import { chatTitlebarMacInsetClass, chatTitlebarRowClass, usesNativeTitlebar } from '../platform'
import { WorkspaceActionsMenu } from '../dock/WorkspaceActionsMenu'
import { ConversationHistoryBranches, type HistoryBranchActions } from '../ConversationHistoryBranches'
import type { Lang } from '../../components/i18n'
import type { AgentRuntimeConfig, Conversation, DetectedExternalAgent, ThinkingLevel } from '../types'

type PopoutTitlebarProps = {
  conversation: Conversation | null
  runtime: AgentRuntimeConfig
  externalAgent?: DetectedExternalAgent
  usesExternalRuntime: boolean
  approvalPolicy: string
  onRuntimeChange: (runtime: AgentRuntimeConfig) => void | Promise<void>
  onModelChange: (providerId: string, model: string) => void | Promise<void>
  onExternalModelChange: ExternalModelChange
  onThinkingLevelChange: (level: ThinkingLevel | null) => void | Promise<void>
  onApprovalPolicyChange: (policy: string) => void | Promise<void>
  onSandboxChange: (value: string) => void | Promise<void>
  busy: boolean
  /** 关联项目且有工作目录时显示工作区面板；弹出窗口始终以浮层打开。 */
  workdir: string
  projectName: string | null
  historyBranches?: HistoryBranchActions | null
  lang: Lang
}

function TitlebarPills({
  conversation,
  runtime,
  externalAgent,
  usesExternalRuntime,
  approvalPolicy,
  onRuntimeChange,
  onModelChange,
  onExternalModelChange,
  onThinkingLevelChange,
  onApprovalPolicyChange,
  onSandboxChange,
  busy,
  workdir,
  projectName,
  historyBranches,
  lang,
}: PopoutTitlebarProps) {
  const firstUser = conversation?.messages?.find((message) => message.role === 'user')
  const titleFallback = conversationTitleSource(
    firstUser?.content ?? '',
    firstUser?.attachments?.map((attachment) => attachment.name) ?? [],
  )
  const title = displayConversationTitle(conversation?.title, titleFallback) || 'Kivio'
  const providerId = conversation?.provider_id ?? ''
  const model = conversation?.model ?? ''
  const showWorkspace = runtime.kind !== 'chat'

  return (
    <>
      <div className="flex min-w-0 items-center gap-1" data-popout-pills>
        <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
          <AgentModelPicker
            agentRuntime={runtime}
            conversationId={conversation?.id}
            locked={!!conversation?.runtime_locked}
            busy={busy}
            providerId={providerId}
            model={model}
            thinkingLevel={conversation?.thinking_level ?? conversation?.thinkingLevel ?? null}
            onRuntimeChange={onRuntimeChange}
            onModelChange={onModelChange}
            onExternalModelChange={onExternalModelChange}
            onThinkingLevelChange={onThinkingLevelChange}
          />
        </div>
        <div className="shrink-0" data-tauri-drag-region="false">
          <PermissionPicker
            agentRuntime={runtime}
            externalAgent={externalAgent}
            approvalPolicy={approvalPolicy}
            onApprovalPolicyChange={onApprovalPolicyChange}
            onSandboxChange={onSandboxChange}
            sandboxLocked={busy}
          />
        </div>
        {usesExternalRuntime && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <ExternalCliAccount agent={externalAgent} conversation={conversation} />
          </div>
        )}
      </div>
      <div className="min-w-4 flex-1 self-stretch" data-tauri-drag-region />
      <div
        data-popout-title
        data-tauri-drag-region
        className="min-w-0 max-w-[42%] shrink truncate px-2 text-right text-[13px] font-medium text-neutral-700"
        title={title}
      >
        {title}
      </div>
      {!showWorkspace && conversation && historyBranches && (
        <ConversationHistoryBranches conversationId={conversation.id} lang={lang} {...historyBranches} />
      )}
      {showWorkspace && (
        <WorkspaceActionsMenu
          workdir={workdir}
          projectName={projectName}
          conversationId={conversation?.id ?? null}
          dockOpen={false}
          lang={lang}
          inlineAllowed={false}
          historyBranches={historyBranches}
        />
      )}
    </>
  )
}

export function PopoutTitlebar(props: PopoutTitlebarProps) {
  if (!usesNativeTitlebar) {
    return (
      <ChatTitlebar
        sidebarExpanded={false}
        hideNav
        onToggleSidebar={() => {}}
        onNewConversation={() => {}}
      >
        <TitlebarPills {...props} />
      </ChatTitlebar>
    )
  }

  return (
    <header
      className={`chat-titlebar-row ${chatTitlebarRowClass} min-w-0 gap-2 ${chatTitlebarMacInsetClass} chat-titlebar-row--collapsed-mac pr-3`}
      data-tauri-drag-region
    >
      <TitlebarPills {...props} />
    </header>
  )
}
