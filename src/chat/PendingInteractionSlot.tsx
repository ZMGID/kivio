import { memo, useMemo } from 'react'
import { ApprovalCard } from './ApprovalCard'
import { AskUserBlock } from './AskUserBlock'
import type { RunInteractionSnapshot } from './runInteractionInbox'
import { userPromptEventToRecord } from './streamApply'
import { buildToolApprovalActions, toolApprovalTitle } from './toolApproval'
import type { AgentRuntimeConfig } from './types'

export interface PendingInteractionSlotProps {
  snapshot: RunInteractionSnapshot
  activeAgentRuntime: AgentRuntimeConfig
  onResolveToolConfirm: (approved: boolean, always?: boolean, permissionMode?: string | null) => Promise<boolean>
  onResolveSessionConsent: (granted: boolean) => Promise<boolean>
  onDismissUserPrompt: (conversationId: string, runId: string, toolCallId: string) => void
  onPersistApprovedSandbox: (conversationId: string, runtime: AgentRuntimeConfig, sandbox: string) => Promise<void>
}

/**
 * 消息列表底部的「待答复」槽：工具审批卡 / 会话授权卡 / 面板式追问。
 * 全部读 run 交互收件箱的快照；没有待答复时不渲染。
 */
export const PendingInteractionSlot = memo(function PendingInteractionSlot({
  snapshot,
  activeAgentRuntime,
  onResolveToolConfirm,
  onResolveSessionConsent,
  onDismissUserPrompt,
  onPersistApprovedSandbox,
}: PendingInteractionSlotProps) {
  const {
    toolConfirm, toolConfirmSubmitting, toolConfirmError,
    sessionConsent, sessionConsentSubmitting, sessionConsentError,
    userPrompt,
  } = snapshot

  /** 面板用的工具记录：**必须记忆** —— 每渲染新建一个对象，会把卡片里
   *  「换了新询问就重置草稿」的 effect 变成每渲染都重置（用户选到一半的答案被清空）。 */
  const userPromptRecord = useMemo(
    () => (userPrompt ? userPromptEventToRecord(userPrompt) : null),
    [userPrompt],
  )

  const toolActions = useMemo(() => (toolConfirm
    ? buildToolApprovalActions(toolConfirm, toolConfirmSubmitting, {
      resolve: onResolveToolConfirm,
      persistSandbox: (mode) => onPersistApprovedSandbox(toolConfirm.conversationId, activeAgentRuntime, mode),
    })
    : []), [activeAgentRuntime, onPersistApprovedSandbox, onResolveToolConfirm, toolConfirm, toolConfirmSubmitting])

  if (!toolConfirm && !sessionConsent && !userPrompt) return null

  return (
    <div className="shrink-0 px-6">
      <div className="mx-auto w-full max-w-4xl">
        {userPrompt && userPromptRecord && (
          <AskUserBlock
            variant="docked"
            toolCall={userPromptRecord}
            onResolved={() => onDismissUserPrompt(userPrompt.conversationId, userPrompt.runId, userPrompt.toolCallId)}
          />
        )}
        {toolConfirm && (
          <ApprovalCard
            title={toolApprovalTitle(toolConfirm)}
            subtitle={`${toolConfirm.source}${toolConfirm.serverId ? ` · ${toolConfirm.serverId}` : ''}`}
            detail={toolConfirm.argumentsPreview}
            error={toolConfirmError}
            actions={toolActions}
          />
        )}
        {sessionConsent && (
          <ApprovalCard
            title="允许本次会话使用文件和命令工具？"
            subtitle="授权后，本会话内 Kivio 可读写、删除磁盘上的任意文件并执行任意终端命令（包括项目目录之外）。仅本次会话有效，重启后需重新授权。"
            error={sessionConsentError}
            actions={[
              {
                label: '拒绝',
                disabled: sessionConsentSubmitting,
                onSelect: () => { void onResolveSessionConsent(false) },
              },
              {
                label: '允许本次会话',
                primary: true,
                hint: 'Ctrl+↵',
                disabled: sessionConsentSubmitting,
                onSelect: () => { void onResolveSessionConsent(true) },
              },
            ]}
          />
        )}
      </div>
    </div>
  )
})
