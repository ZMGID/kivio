import type { ChatSessionConsentPayload, ChatToolConfirmPayload, ChatUserPromptPayload } from '../api/tauri'

/**
 * 一个会话在前端持有的待交互状态。执行身份由 chatExecutionOwner 独占，
 * 流预览由 streamPreviewOwner 独占。
 *
 * 这些字段原本是 Chat.tsx 里多个独立的 ref。「清理一个会话」
 * 必须同时动其中若干个，Chat.tsx 里因此出现了 6 处手写的删除块，字段组合各不相同
 * （[CST] 三处、[CDEFST]、[CEST]、[CT]），差异全靠人记。
 *
 * 这里只把「清理」这个动作收敛成显式的谓词，ref 本身仍留在 Chat.tsx；
 * 延迟 run 终态已由 chatExecutionOwner 独占，流预览由 streamPreviewOwner 独占。
 */
export interface ConversationLocalState {
  streamErrors: Record<string, string>
  /**
   * 每会话一条待审批队列（不是单个）。claude 会在一条消息里并行调多个工具，
   * 后端也是按 request_id 并发挂着等的；这里若只留一个槽位，第二条询问会覆盖第一条，
   * 用户没看见的那条会在后端超时后被判成「用户拒绝」。
   */
  pendingToolConfirms: Record<string, ChatToolConfirmPayload[]>
  pendingSessionConsents: Record<string, ChatSessionConsentPayload>
  /** 每会话一条待答的问用户询问队列（面板吊在输入框上方，同审批卡的并发理由）。 */
  pendingUserPrompts: Record<string, ChatUserPromptPayload[]>
}

/** 清理时可选择动哪些字段。默认只清「一轮结束」必然要清的三项。 */
export interface ClearScope {
  /** 同时清错误（会话被删除时；正常结束要保留以便展示失败原因）。 */
  streamErrors?: boolean
}

/**
 * 清掉一个会话的本地运行态。
 *
 * 无条件清理的三项 —— 待确认工具、待确认会话授权、待答询问 —— 是「这一轮跑完了」
 * 的定义，6 处调用点全都要清。可选项按场景开启，见 ClearScope 各字段注释。
 */
export function clearConversationLocalState(
  state: ConversationLocalState,
  conversationId: string,
  scope: ClearScope = {},
): void {
  delete state.pendingToolConfirms[conversationId]
  delete state.pendingSessionConsents[conversationId]
  delete state.pendingUserPrompts[conversationId]
  if (scope.streamErrors) delete state.streamErrors[conversationId]
}
