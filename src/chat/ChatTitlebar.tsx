import { memo, useState, type ReactNode } from 'react'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { WindowControls } from './WindowControls'
import { IconButton } from '../components/Button'
import { useT } from '../components/i18n'
import { SidebarToggleIcon } from './SidebarToggleIcon'
import { usePanelShiftMotion } from './usePanelShiftMotion'

type ChatTitlebarProps = {
  sidebarExpanded: boolean
  onToggleSidebar: () => void
  onNewConversation: () => void
  /** 侧栏卡片当前是否占位。true 时业务控件让到主区那一列，与主内容左缘对齐。 */
  sidebarVisible?: boolean
  /**
   * 设置页语境：带改为浮在内容之上（不占行高），左段不放按钮。
   *
   * 设置页的 200px 灰导航栏是齐窗口左缘的实心列，带若仍是独立一行，灰栏上方会横着
   * 一条主背景白带 —— 那就是顶栏「突兀」的来源。带浮起后导航栏自己从 y=0 画起，
   * 宽度与底色全归它一家管；这很关键，因为它的宽度由 `@container settings-shell`
   * 收窄（200→52px），而容器在内容行内，带在容器外收不到那个查询，无法跟随。
   *
   * 左段留空：「侧栏开合 / 新建聊天」在设置页无意义（聊天侧栏已被导航栏顶掉），
   * 「返回对话」导航栏底部已有一枚，不重复放。带保持全宽透明，故窗口拖拽区不丢。
   */
  settingsMode?: boolean
  /** 弹出窗：不渲染侧栏开合 / 新建，带仍占行高（不要用 settingsMode，那会浮到正文上）。 */
  hideNav?: boolean
  dockVisible?: boolean
  dockWidth?: number
  dockToolbarRef?: (node: HTMLDivElement | null) => void
  onToggleDock?: () => void
  /** 会话页的业务控件（模型 / 思考档位 / 权限 / Dock 等）。中心页传 null，带内只剩导航与三键。 */
  children?: ReactNode
}

/**
 * Windows / Linux 的全宽标题栏带（macOS 走系统 Overlay 标题栏，不渲染此组件）。
 *
 * 贯穿窗口全宽：左端常驻「侧栏开合 + 新建聊天」，中段是会话页业务控件，右端贴角 caption 键。
 * 业务控件并进这一条 —— 非 mac 不再有主区 52px 顶栏，chrome 只此一行。
 * 侧栏展开时导航区撑到侧栏宽，业务控件随之让到主区一列（否则它们压在侧栏卡片上方，读作错位）。
 * Dock 打开时工具栏并入右段，右段与下方面板同宽；其余内容从标题栏下方开始。
 *
 * 侧栏 / Dock 开合时，布局和下方一样只切换一次：两列的底色与分隔线由 `chat-titlebar-panel-bg`
 * 独立淡入淡出（与面板同时长、同位移），业务控件从旧左缘平移到新左缘（usePanelShiftMotion）。
 */
export const ChatTitlebar = memo(function ChatTitlebar({
  sidebarExpanded,
  onToggleSidebar,
  onNewConversation,
  sidebarVisible = false,
  settingsMode = false,
  hideNav = false,
  dockVisible = false,
  dockWidth,
  dockToolbarRef,
  onToggleDock,
  children,
}: ChatTitlebarProps) {
  const t = useT()
  const [strip, setStrip] = useState<HTMLDivElement | null>(null)
  usePanelShiftMotion(strip, sidebarVisible ? '1' : '0', settingsMode ? 'settings' : 'main', 'start')
  // 进设置页时侧栏直接让位（见 .is-settings-cover），底色也不在设置页上方淡出。
  const panelBgClass = (side: 'sidebar' | 'dock', visible: boolean) =>
    `chat-titlebar-panel-bg chat-titlebar-panel-bg--${side}${visible ? ' is-visible' : ''}${settingsMode ? ' is-instant' : ''}`
  return (
    <div
      ref={setStrip}
      className={`chat-titlebar-strip${settingsMode ? ' chat-titlebar-strip--settings' : ''}${hideNav && !settingsMode ? ' chat-titlebar-strip--popout' : ''}`}
      data-tauri-drag-region
    >
      {!hideNav && <div aria-hidden className={panelBgClass('sidebar', sidebarVisible && !settingsMode)} />}
      {!settingsMode && !hideNav && (
        <div
          className={`chat-titlebar-strip-nav${sidebarVisible ? ' chat-titlebar-strip-nav--reserve' : ''}`}
          data-tauri-drag-region
        >
          <ChatTitlebarActions
            sidebarExpanded={sidebarExpanded}
            onToggleSidebar={onToggleSidebar}
            onNewConversation={onNewConversation}
          />
        </div>
      )}
      <div
        className="chat-titlebar-strip-content chat-panel-shift-host flex min-w-0 flex-1 self-stretch"
        data-tauri-drag-region
      >
        <div className="chat-titlebar-strip-controls flex min-w-0 flex-1 items-center gap-1" data-tauri-drag-region>
          {children}
        </div>
      </div>
      <div
        className={`chat-titlebar-strip-end${dockVisible ? ' chat-titlebar-strip-dock' : ''}`}
        /* 收起时也保留宽度变量：Dock 底色要按原宽度淡出。拖拽时 RightDock 直写这一节点。 */
        style={dockWidth != null ? { ['--chat-dock-width' as string]: `${dockWidth}px` } : undefined}
        data-tauri-drag-region
      >
        {onToggleDock && <div aria-hidden className={panelBgClass('dock', dockVisible)} />}
        {dockVisible && <div ref={dockToolbarRef} className="chat-titlebar-dock-tools" data-tauri-drag-region />}
        {onToggleDock && <IconButton label={t.dockToggle} size="sm" variant="ghost" className="group" aria-expanded={dockVisible} onClick={onToggleDock} data-tauri-drag-region="false">
          <SidebarToggleIcon side="right" open={dockVisible} />
        </IconButton>}
        <WindowControls />
      </div>
    </div>
  )
})
