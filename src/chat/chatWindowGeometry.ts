import { CHAT_MIN_SIZE_COLLAPSED } from './persistence'

/** 侧栏折叠 / 展开时聊天窗的逻辑像素 min-size。 */
export function chatWindowMinSize(collapsed: boolean, sidebarWidth: number): { width: number; height: number } {
  if (collapsed) return CHAT_MIN_SIZE_COLLAPSED
  return {
    width: CHAT_MIN_SIZE_COLLAPSED.width + sidebarWidth,
    height: CHAT_MIN_SIZE_COLLAPSED.height,
  }
}
