import type { ComponentType, ReactNode } from 'react'
import type { Conversation, PendingAttachment, InputBarProps, MessageListProps } from './chat'

export const EXTENSION_API_VERSION = 1
export interface ExtensionManifest {
  id: string
  name: string
  description: string
  version: string
  apiVersion: number
  /** Historical routes are redirects only; they never bypass activation. */
  legacyRoutes?: string[]
}
export interface ExtensionPageProps {
  headerControls?: ReactNode
  inputBarProps: InputBarProps
  messageListProps: MessageListProps
  modelControls: ReactNode
  selectedModel?: { providerId: string; model: string }
  onCreateConversation: (title: string) => Promise<Conversation>
  onActivateConversation: (id: string | null) => Promise<void>
  onSend: (conversation: Conversation, content: string, attachments: PendingAttachment[], onAccepted?: () => void) => Promise<boolean>
}
export interface ExtensionContext {
  readonly extensionId: string
  /** Aborted when this activation is disposed. Never reuse a captured context. */
  readonly signal: AbortSignal
}
export interface ExtensionHookMap {
  beforeDisable: {
    event: { reason: 'user'; extensionId: string }
    result: { cancel: true; reason: string } | void
  }
  activated: { event: { reason: 'enable' | 'restore'; extensionId: string }; result: void }
  deactivated: { event: { reason: 'user'; extensionId: string }; result: void }
}
export type HookHandler<K extends keyof ExtensionHookMap> = (
  event: Readonly<ExtensionHookMap[K]['event']>, context: ExtensionContext,
) => ExtensionHookMap[K]['result'] | Promise<ExtensionHookMap[K]['result']>
export interface ExtensionAPI {
  readonly context: ExtensionContext
  /** Registration order is execution order; unsubscribe is idempotent. */
  on<K extends keyof ExtensionHookMap>(name: K, handler: HookHandler<K>): () => void
}
export interface ExtensionModule {
  Page: ComponentType<ExtensionPageProps>
  /** Register hooks only. Long-running work belongs to activation or explicit actions. */
  activate(api: ExtensionAPI): void | Promise<void>
}
export function defineExtension(extension: ExtensionModule): ExtensionModule { return extension }
