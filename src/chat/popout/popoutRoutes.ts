import { hashPath } from '../browserRoute'
import { decodeChatRouteId, encodeChatRouteId } from '../routeCodec'

export { isChatPopoutPath } from '../routeCodec'

export function popoutConversationHash(conversationId: string): string {
  return `#${encodeChatRouteId('chat/popout/', conversationId)}`
}

export function getPopoutConversationIdFromPath(path: string): string | null {
  return decodeChatRouteId('chat/popout/', path)
}

export function getPopoutConversationId(): string | null {
  return getPopoutConversationIdFromPath(hashPath())
}
