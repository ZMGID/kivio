import { useEffect, useRef, type RefObject } from 'react'

const ITEM_SELECTOR = [
  'button:not([disabled])',
  'input:not([disabled])',
  '[role="menuitem"]:not([aria-disabled="true"])',
  '[role="menuitemradio"]:not([aria-disabled="true"])',
  '[role="menuitemcheckbox"]:not([aria-disabled="true"])',
].join(', ')

function menuItems(menu: HTMLElement): HTMLElement[] {
  return [...menu.querySelectorAll<HTMLElement>(ITEM_SELECTOR)]
    // role 选择器也会命中带 disabled 的 <button>；它 focus() 不动，会把方向键卡住。
    .filter((el) => !el.hasAttribute('disabled') && !el.closest('[inert], [hidden], [aria-hidden="true"]'))
}

function openedWithKeyboard(el: Element | null): boolean {
  try {
    return Boolean(el?.matches(':focus-visible'))
  } catch {
    return false
  }
}

/**
 * 标题栏 / 输入栏下拉菜单的键盘行为，打开期间生效：
 * - Esc 关闭；焦点在菜单里时还给打开它的按钮（否则菜单卸载后焦点掉到 body）。
 * - 传了 menuRef 时：键盘打开（按钮处于 :focus-visible）直接聚焦当前选中项（没有则首项）；
 *   ↑ / ↓ / Home / End 在菜单项间移动，焦点在按钮上时 ↓ / ↑ 进入菜单。鼠标打开不抢焦点。
 * 触发按钮不用传：打开那一刻的 activeElement 就是它。
 * Esc 的默认行为被取消，避免同时关闭承载菜单的原生 dialog。
 */
export function usePopoverMenu(
  open: boolean,
  onClose: () => void,
  menuRef?: RefObject<HTMLElement | null>,
): void {
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose
  useEffect(() => {
    if (!open) return
    const opener = document.activeElement instanceof HTMLElement && document.activeElement !== document.body
      ? document.activeElement
      : null
    // 菜单元素按需读：有的弹层（portal + 先量位置）要晚一帧才挂上。
    const getMenu = () => menuRef?.current ?? null
    let frame = 0
    if (menuRef && openedWithKeyboard(opener)) {
      const focusFirst = () => {
        const menu = getMenu()
        if (menu) {
          // 有「当前选中」项就落在它上面（和原生菜单一致），否则落首项。
          const items = menuItems(menu)
          const selected = items.find((el) => el.getAttribute('aria-checked') === 'true'
            || el.getAttribute('aria-selected') === 'true'
            || el.getAttribute('aria-current') === 'true')
          ;(selected ?? items[0])?.focus()
        }
        return Boolean(menu)
      }
      if (!focusFirst()) frame = requestAnimationFrame(() => { focusFirst() })
    }

    const onKey = (event: KeyboardEvent) => {
      const menu = getMenu()
      const active = document.activeElement
      if (event.key === 'Escape') {
        event.preventDefault()
        const focusInMenu = Boolean(menu && active && menu.contains(active))
        onCloseRef.current()
        if (focusInMenu) opener?.focus()
        return
      }
      if (!menu || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
      const inMenu = Boolean(active && menu.contains(active))
      if (!inMenu && active !== opener) return
      // 输入框里 Home / End 是移动光标，不接管。
      if (inMenu && active instanceof HTMLInputElement && (event.key === 'Home' || event.key === 'End')) return
      const items = menuItems(menu)
      if (items.length === 0) return
      event.preventDefault()
      const index = inMenu ? items.indexOf(active as HTMLElement) : -1
      let next: number
      if (event.key === 'Home') next = 0
      else if (event.key === 'End') next = items.length - 1
      else if (event.key === 'ArrowDown') next = index < 0 ? 0 : (index + 1) % items.length
      else next = index < 0 ? items.length - 1 : (index - 1 + items.length) % items.length
      items[next]?.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      cancelAnimationFrame(frame)
      window.removeEventListener('keydown', onKey)
    }
  }, [open, menuRef])
}
