import { memo, useEffect, useState } from 'react'
import { StreamDotLogo } from './StreamDotLogo'

/** 目标会话数据迟迟未到时，多久后才盖上 Logo。须明显长于常见读取耗时（实测 4–40ms）。 */
const DELAYED_LOGO_MS = 400

/**
 * 会话切换的慢加载反馈。列表跨会话复用，读取期间继续显示上一会话；只有读取超过
 * 400ms 才铺底色和 Logo，避免每次切换都「刷新一下」，又不让慢读取看起来像卡死。
 */
export const ConversationLoadingState = memo(function ConversationLoadingState() {
  const [visible, setVisible] = useState(false)
  useEffect(() => {
    const timer = window.setTimeout(() => setVisible(true), DELAYED_LOGO_MS)
    return () => window.clearTimeout(timer)
  }, [])
  if (!visible) return null
  return (
    <div
      className="chat-conversation-loading absolute inset-0 z-30 flex items-center justify-center"
      role="status"
      aria-label="正在加载对话"
    >
      <StreamDotLogo size={104} />
      <span className="sr-only">正在加载对话…</span>
    </div>
  )
})
