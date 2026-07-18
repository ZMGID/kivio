import { useCallback, useRef, useState } from 'react'
import type { AnimationEvent } from 'react'

/**
 * 让浮层「关闭时」先播退场动画，再真正卸载——无需改父组件的 `{open && <X/>}` 挂载逻辑。
 *
 * 用法：组件内部把所有关闭触发（Esc / 点外部 / 取消按钮 / 菜单项动作后）改成调用 `startClose`，
 * 根元素在 `closing` 时挂退场 class（如 chat-motion-modal-out / chat-motion-popover-out）并绑定
 * `onAnimationEnd={onAnimationEnd}`。动画结束（或 `fallbackMs` 超时兜底，防 animationend 不触发时
 * 浮层关不掉）后调用真实 `onClose`，父级再卸载。
 *
 * reduced-motion 下退场动画被全局兜底压到 0.01ms，animationend 仍会触发，卸载不卡。
 */
export function useCloseAnimation(onClose: () => void, fallbackMs = 320) {
  const [closing, setClosing] = useState(false)
  const doneRef = useRef(false)

  const finish = useCallback(() => {
    if (doneRef.current) return
    doneRef.current = true
    onClose()
  }, [onClose])

  const startClose = useCallback(() => {
    setClosing((c) => {
      if (c) return c
      window.setTimeout(finish, fallbackMs)
      return true
    })
  }, [finish, fallbackMs])

  const onAnimationEnd = useCallback(
    (e: AnimationEvent) => {
      // 只认根元素自身的退场动画，忽略子元素冒泡上来的 animationend
      if (e.target === e.currentTarget && closing) finish()
    },
    [closing, finish],
  )

  return { closing, startClose, onAnimationEnd }
}
