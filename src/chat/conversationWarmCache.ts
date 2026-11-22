import type { ChatSyncCheckpoint } from '../api/chatProtocol'
import type { ChatHistoryCursor, ChatHistoryReplay } from '../generated/chatProtocol'
import type { Conversation, ConversationContextState } from './types'

function applyReplay(window: Conversation, replay: ChatHistoryReplay): Conversation | null {
  if (replay.after.revision !== window.revision || replay.after.source !== window.history_source) return null
  if (!replay.patches.length && replay.through.source !== replay.after.source) return null
  let current = window
  for (const patch of replay.patches) {
    if (patch.revision !== current.revision + 1) return null
    current = { ...current, revision: patch.revision, updated_at: patch.updatedAt }
    for (const event of patch.events) {
      switch (event.type) {
        case 'title_updated':
          current.title = event.title
          break
        case 'todo_updated':
          current.agent_todo_state = current.agentTodoState = {
            ...event.todoState, items: event.todoState.items.map(item => ({ ...item, blocked_by: item.blockedBy })),
          }
          break
        case 'plan_updated':
          current.agent_plan_state = current.agentPlanState = event.planState
          break
        case 'context_updated':
          current.context_state = current.contextState = event.contextState as ConversationContextState
          break
        case 'goal_updated':
          current.goal_state = current.goalState = event.goalState
          break
        case 'subagent_activity_added':
          return null
      }
    }
  }
  if (current.revision !== replay.through.revision) return null
  return current.history_source === replay.through.source ? current : { ...current, history_source: replay.through.source }
}

const IDLE_TTL_MS = 5 * 60_000

/** Keeps inactive display windows for the same five-minute idle lifetime as T3 threads. */
type RetainedWindow = {
  conversation: Conversation
  bounding?: Promise<void>
  owner: symbol
  timer: ReturnType<typeof setTimeout>
  stillCurrent?: () => boolean
}

export function createConversationWarmCache(readWindow: (id: string, anchor?: string) => Promise<Conversation>) {
  const entries = new Map<string, RetainedWindow>()

  const forget = (id: string) => {
    const entry = entries.get(id)
    if (entry) clearTimeout(entry.timer)
    entries.delete(id)
  }

  return {
    remember(conversation: Conversation, anchor?: string, stillCurrent?: () => boolean) {
      const id = conversation.id
      forget(id)
      const owner = Symbol()
      const entry: RetainedWindow = { owner, conversation, stillCurrent, timer: setTimeout(() => forget(id), IDLE_TTL_MS) }
      entries.set(id, entry)
      if (conversation.history_start != null && conversation.history_source) return
      // Full mutation responses can be displayed on an immediate return. Release their
      // large tool payloads as soon as the stored display window is available.
      entry.bounding = readWindow(id, anchor).then(window => {
        const current = entries.get(id)
        if (current?.owner === owner) current.conversation = window
      }, () => {
        if (entries.get(id)?.owner === owner) forget(id)
      })
    },
    async get(
      id: string,
      resume: (id: string, after?: ChatHistoryCursor) => Promise<ChatSyncCheckpoint | undefined>,
      display: (conversation: Conversation) => void,
    ): Promise<Conversation | null> {
      const entry = entries.get(id)
      if (!entry) return null
      display(entry.conversation)
      if (entry.bounding) await entry.bounding
      if (entries.get(id) !== entry) return null
      const base = entry.conversation
      const after = base.history_source ? { revision: base.revision, source: base.history_source } : undefined
      const checkpoint = await resume(id, after)
      if (entries.get(id) !== entry) return null
      if (checkpoint?.conversationRevision === null) {
        forget(id)
        throw new Error('Conversation no longer exists')
      }
      const replay = checkpoint?.historyReplay
      const current = replay && replay.through.revision === checkpoint.conversationRevision ? applyReplay(base, replay) : null
      if (!current || (entry.stillCurrent && !entry.stillCurrent())) {
        forget(id)
        return null
      }
      return current
    },
    forget,
    clear() { for (const id of entries.keys()) forget(id) },
  }
}
