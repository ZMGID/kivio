import { useMemo } from 'react'
import type { CliAccountScope } from '../api/externalCliSettings'
import type { Conversation } from './types'

export type CliAccountConversation = Pick<Conversation, 'id' | 'project_id' | 'projectId' | 'folder' | 'worktree'>

export function useChatCliAccountScope(conversation?: CliAccountConversation | null, draftProjectId?: string | null): CliAccountScope {
  const projectId = conversation ? conversation.project_id ?? conversation.projectId : draftProjectId
  // Worktrees and legacy project names need the backend's conversation-specific lookup.
  const savedId = conversation?.worktree || (conversation?.folder && !projectId) ? conversation.id : null
  return useMemo(() => savedId ?? (projectId ? { projectId } : null), [savedId, projectId])
}
