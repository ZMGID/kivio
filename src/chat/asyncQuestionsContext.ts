import { createContext } from 'react'

/** Async CLI questions reply through the normal user-message queue, not tool approval. */
export const AsyncQuestionsContext = createContext<{
  closedIds: ReadonlySet<string>
  reply: (toolId: string, text: string | null) => Promise<void>
} | null>(null)
