import { useEffect, useRef } from 'react'

// 弹层打开期间按 Esc 关闭。只在 open 时挂监听；onClose 走 ref，调用方传内联箭头也不会反复重挂。
// 不拦截冒泡：生成中「Esc = 停止」只绑在输入框上，点开菜单后焦点在触发按钮，不会连带触发。
export function useEscapeToClose(open: boolean, onClose: () => void): void {
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose
  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onCloseRef.current()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])
}
