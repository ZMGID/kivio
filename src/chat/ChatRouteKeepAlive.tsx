import { useRef, type ReactNode } from 'react'
import { ChatRouteActiveContext } from './chatRouteActiveContext'

interface ChatRouteKeepAliveProps {
  activeKey: string
  settingsExiting?: boolean
  children: ReactNode
}
/**
 * 保留聊天页挂载状态，返回时复用草稿与滚动位置。
 *
 * 聊天历史包含较大的 React/Markdown 子树。路由分支直接互斥渲染时，
 * 从设置返回聊天会让 MessageList 从零挂载，因此只缓存 conversation。
 * 设置页由关闭流程等待保存并完成退场后卸载；绕过关闭流程时保留未确认的草稿。
 *
 * Hidden panes must be a real `display:none` box (`hidden` + `inert`).
 * `display: contents` lets a cached conversation composer leak into the
 * settings layout and swallow the back-button click.
 */
export function ChatRouteKeepAlive({ activeKey, settingsExiting = false, children }: ChatRouteKeepAliveProps) {
  const cacheRef = useRef(new Map<string, ReactNode>())
  const settingsCanRelease = useRef(false)
  // External navigation can bypass the settings save gate; retain its draft in that case.
  if (activeKey === 'settings') settingsCanRelease.current = settingsExiting
  else if (settingsCanRelease.current) cacheRef.current.delete('settings')
  const keep = activeKey === 'conversation' || activeKey === 'settings'
  if (keep) cacheRef.current.set(activeKey, children)

  return (
    // 页面自身会淡入/位移；阅读底色留在常驻层，避免半透明窗口在切换时露出原生材质。
    // chat-panel-shift-host：侧面板开合时主区平移动画的宿主（见 usePanelShiftMotion）。
    <div className="chat-panel-shift-host chat-themed-surface flex min-h-0 min-w-0 flex-1">
      {[...cacheRef.current.entries()].map(([key, node]) => {
        const active = key === activeKey
        return (
          <div
            key={key}
            className={active ? 'flex min-h-0 min-w-0 flex-1' : undefined}
            hidden={!active}
            aria-hidden={active ? undefined : true}
            {...(active ? {} : { inert: '' })}
          >
            <ChatRouteActiveContext.Provider value={active}>{node}</ChatRouteActiveContext.Provider>
          </div>
        )
      })}
      {!keep && <div className="flex min-h-0 min-w-0 flex-1">{children}</div>}
    </div>
  )
}
