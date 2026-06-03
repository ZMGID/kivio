export const isMac =
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent)

/** macOS Chat 使用 Tauri Overlay 标题栏，交通灯由系统绘制 */
export const usesNativeTitlebar = isMac

/** 侧栏 / 主内容顶栏行高与垂直居中 */
export const chatTitlebarRowClass = usesNativeTitlebar
  ? 'flex h-[52px] shrink-0 items-center gap-2'
  : 'flex h-[52px] shrink-0 items-center gap-2 px-3 pt-2'

/** 窗口左缘交通灯留白（仅侧栏顶栏、收起态主顶栏；约 66px 灯区 + 间距） */
export const chatTitlebarMacInsetClass = usesNativeTitlebar ? 'pl-[92px]' : ''

/** 顶栏胶囊基础样式 */
export const chatTitlebarPillClass =
  'inline-flex shrink-0 items-center rounded-full border border-neutral-200/90 bg-white shadow-sm dark:border-neutral-700 dark:bg-neutral-900'

export const chatTitlebarPillHoverClass = 'hover:bg-neutral-50 dark:hover:bg-neutral-800'

/** 顶栏胶囊控件统一尺寸（模型选择、侧栏操作等） */
export const chatTitlebarPillButtonClass = [
  chatTitlebarPillClass,
  'h-[34px] gap-1.5 px-3 text-sm transition-colors',
  chatTitlebarPillHoverClass,
].join(' ')

/** 顶栏胶囊内的图标按钮（无额外外框，避免撑高） */
export const chatTitlebarPillIconClass =
  'flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-neutral-600 transition-colors hover:bg-black/[0.05] hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-white/[0.08] dark:hover:text-neutral-100'
