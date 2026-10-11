import { api } from '../api/tauri'
import { isBotOwnedConversation } from './botIdentity'
import { isTauriRuntime } from './utils'

/** The active workspace is explicit in the URL, separate from sidebar layout. */
export function isBotWorkspace(): boolean {
  return typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('workspace') === 'bot'
}

export function belongsToWorkspace(conversationId: string): boolean {
  return isBotOwnedConversation(conversationId) === isBotWorkspace()
}

export function workspaceStorageKey(key: string): string {
  return isBotWorkspace() ? `bots:${key}` : key
}

export async function switchChatWorkspace(bot: boolean): Promise<void> {
  if (isTauriRuntime()) return api.switchChatWorkspace(bot)
  const url = new URL(window.location.href)
  window.sessionStorage.setItem(workspaceStorageKey('workspace-route'), url.hash)
  const key = bot ? 'bots:workspace-route' : 'workspace-route'
  url.search = bot ? '?workspace=bot' : ''
  url.hash = window.sessionStorage.getItem(key) || 'chat'
  window.location.replace(url.href)
}
