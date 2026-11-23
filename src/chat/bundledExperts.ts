import type { ChatAssistant } from './types'

export const bundledExperts = Object.values(import.meta.glob<ChatAssistant>(
  '../../src-tauri/resources/experts/*.json', { eager: true, import: 'default' },
)).map((expert): ChatAssistant => ({ ...expert, tool_policy: 'preferences' }))
