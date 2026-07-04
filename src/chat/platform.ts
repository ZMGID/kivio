export const isMac =
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent)

export const isWindows =
  typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent)

/** macOS Chat 使用 Tauri Overlay 標題欄，交通燈由系統繪製 */
export const usesNativeTitlebar = isMac

/** 側欄 / 主內容頂欄行高與垂直居中 */
export const chatTitlebarRowClass = usesNativeTitlebar
  ? 'flex h-[52px] shrink-0 items-center gap-2'
  : 'flex h-[52px] shrink-0 items-center gap-2 px-3 pt-2'

/** 視窗左緣交通燈留白（僅側欄頂欄、收起態主頂欄；約 66px 燈區 + 間距） */
export const chatTitlebarMacInsetClass = usesNativeTitlebar ? 'pl-[92px]' : ''

/**
 * 頂欄幽靈控制元件統一規格（去膠囊化方向 A）：
 * - 預設透明，無邊框 / 無陰影；hover 才出淺背景（與圖示鈕 hover-bg 同語彙）。
 * - 統一高度 32px（h-8）、統一圓角 rounded-lg，所有觸發控制元件對齊到同一視覺規格。
 * - 沿用 active:scale 按壓反饋 + --kv-* 令牌；reduced-motion 靠 index.css 末尾全域兜底。
 */
export const chatTitlebarGhostHoverClass =
  'hover:bg-black/[0.05] dark:hover:bg-white/[0.07]'

/** 文字 / 複合觸發控制元件（模型選擇、側欄操作組容器等）。 */
export const chatTitlebarPillButtonClass = [
  'chat-titlebar-pill',
  'inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-transparent bg-transparent px-2.5 text-sm transition duration-[var(--kv-dur-instant)] active:scale-[0.97]',
  chatTitlebarGhostHoverClass,
].join(' ')

/** 純圖示觸發控制元件（Runtime / Permission 等），與 pill 同高、同圓角的方形鈕。 */
export const chatTitlebarIconButtonClass = [
  'chat-titlebar-pill',
  'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-transparent bg-transparent text-neutral-600 transition duration-[var(--kv-dur-instant)] active:scale-[0.97] dark:text-neutral-400',
  chatTitlebarGhostHoverClass,
].join(' ')

/** 複合控制元件內的次級圖示按鈕（如側欄操作組裡的切欄 / 新建）。 */
export const chatTitlebarPillIconClass =
  'chat-titlebar-pill-icon flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-neutral-600 transition duration-[var(--kv-dur-instant)] hover:bg-black/[0.05] hover:text-neutral-900 active:scale-90 dark:text-neutral-400 dark:hover:bg-white/[0.08] dark:hover:text-neutral-100'
