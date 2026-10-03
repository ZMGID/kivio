import { hashPath } from '../browserRoute'
import { decodeChatRouteId } from '../routeCodec'

export { isChatPopoutPath } from '../routeCodec'

export function getPopoutConversationIdFromPath(path: string): string | null {
  return decodeChatRouteId('chat/popout/', path)
}

export function getPopoutConversationId(): string | null {
  return getPopoutConversationIdFromPath(hashPath())
}
