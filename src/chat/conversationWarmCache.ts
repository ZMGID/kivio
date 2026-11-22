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

const MAX_ENTRIES = 4
const MAX_BYTES = 24 * 1024 * 1024

// Charge strings at UTF-16 width and account for collection slots and field names.
// Walk the already-loaded JSON values without building a serialized copy of tool results.
function retainedBytes(value: unknown, budget = MAX_BYTES, depth = 0): number {
  if (typeof value === 'string') return 16 + value.length * 2
  if (value == null || typeof value !== 'object') return 8
  if (depth > 128) return MAX_BYTES + 1
  let bytes = 32
  if (Array.isArray(value)) {
    for (const item of value) {
      bytes += 8 + retainedBytes(item, budget - bytes, depth + 1)
      if (bytes > budget) break
    }
  } else {
    for (const key in value) {
      if (!Object.hasOwn(value, key)) continue
      bytes += 16 + 2 * key.length + retainedBytes(Reflect.get(value, key), budget - bytes, depth + 1)
      if (bytes > budget) break
    }
  }
  return bytes
}

type RetainedWindow = {
  conversation: Conversation | null
  bounding?: Promise<void>
  bytes: number
  timer: ReturnType<typeof setTimeout>
  stillCurrent?: () => boolean
}

export function createConversationWarmCache(readWindow: (id: string, anchor?: string) => Promise<Conversation>) {
  const entries = new Map<string, RetainedWindow>()

  let usedBytes = 0

  const forget = (id: string) => {
    const entry = entries.get(id)
    if (!entry) return
    clearTimeout(entry.timer)
    usedBytes -= entry.bytes
    entries.delete(id)
    // Pending reads may still hold this entry until their promise settles.
    entry.conversation = null
  }

  const trim = () => {
    for (const id of entries.keys()) {
      if (usedBytes <= MAX_BYTES && entries.size <= MAX_ENTRIES) break
      forget(id)
    }
  }

  return {
    remember(conversation: Conversation, anchor?: string, stillCurrent?: () => boolean) {
      const id = conversation.id
      forget(id)
      const bytes = retainedBytes(conversation)
      if (bytes > MAX_BYTES) return
      const entry: RetainedWindow = { conversation, bytes, stillCurrent, timer: setTimeout(() => forget(id), IDLE_TTL_MS) }
      entries.set(id, entry)
      usedBytes += bytes
      trim()
      if (conversation.history_start != null && conversation.history_source) return
      entry.bounding = readWindow(id, anchor).then(window => {
        if (entries.get(id) !== entry) return
        const bytes = retainedBytes(window)
        if (bytes > MAX_BYTES) {
          forget(id)
          return
        }
        usedBytes += bytes - entry.bytes
        entry.bytes = bytes
        entry.conversation = window
        trim()
      }, () => {
        if (entries.get(id) === entry) forget(id)
      })
    },
    async get(
      id: string,
      resume: (id: string, after?: ChatHistoryCursor) => Promise<ChatSyncCheckpoint | undefined>,
      display: (conversation: Conversation) => void,
    ): Promise<Conversation | null> {
      const entry = entries.get(id)
      if (!entry) return null
      if (entry.conversation) display(entry.conversation)
      if (entry.bounding) await entry.bounding
      if (entries.get(id) !== entry || !entry.conversation) return null
      const after = entry.conversation.history_source
        ? { revision: entry.conversation.revision, source: entry.conversation.history_source } : undefined
      const checkpoint = await resume(id, after)
      if (entries.get(id) !== entry || !entry.conversation) return null
      const base = entry.conversation
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
