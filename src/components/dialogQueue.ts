/**
 * 应用内确认 / 提示框，替代 window.confirm / window.alert：
 * 原生框样式与应用割裂、冻结整个页面（流式输出也停），且无法随界面语言与主题变化。
 *
 * 用法：`if (!(await confirmDialog({ message, confirmLabel, danger: true }))) return`。
 * 需要窗口根部挂一个 <AppDialogHost />（./AppDialog.tsx；它读 LangContext，放在语言 Provider 里面）；
 * 没有宿主时（单元测试、未挂载的窗口）退回原生框，行为与原来一致。
 *
 * 注意与原生框的差异：等待期间事件循环照常运行。确认前做的状态检查，确认后可能已过期，
 * 调用方在执行前按需再查一次。
 */

export type ConfirmDialogOptions = {
  title?: string
  message: string
  confirmLabel?: string
  cancelLabel?: string
  /** 破坏性操作：确认键用危险色，默认焦点落在「取消」上，防止回车误删。 */
  danger?: boolean
}

export type AlertDialogOptions = {
  title?: string
  message: string
  okLabel?: string
}

export type DialogRequest =
  | { id: number; kind: 'confirm'; options: ConfirmDialogOptions; resolve: (value: boolean) => void }
  | { id: number; kind: 'alert'; options: AlertDialogOptions; resolve: (value: boolean) => void }

let nextId = 1
let queue: DialogRequest[] = []
let hostCount = 0
const listeners = new Set<() => void>()

function emit() {
  for (const listener of listeners) listener()
}

export function subscribeDialogs(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function currentDialog(): DialogRequest | null {
  return queue[0] ?? null
}

function enqueue(request: DialogRequest) {
  queue = [...queue, request]
  emit()
}

export function settleDialog(id: number, value: boolean) {
  const request = queue.find((item) => item.id === id)
  if (!request) return
  queue = queue.filter((item) => item.id !== id)
  emit()
  request.resolve(value)
}

export function confirmDialog(options: ConfirmDialogOptions | string): Promise<boolean> {
  const opts = typeof options === 'string' ? { message: options } : options
  if (hostCount === 0) return Promise.resolve(window.confirm(opts.message))
  return new Promise((resolve) => enqueue({ id: nextId++, kind: 'confirm', options: opts, resolve }))
}

export function alertDialog(options: AlertDialogOptions | string): Promise<void> {
  const opts = typeof options === 'string' ? { message: options } : options
  if (hostCount === 0) {
    window.alert(opts.message)
    return Promise.resolve()
  }
  return new Promise((resolve) => enqueue({ id: nextId++, kind: 'alert', options: opts, resolve: () => resolve() }))
}

/** 宿主挂载登记；返回注销函数。最后一个宿主卸载时，未答复的对话框按「取消」结束，避免调用方永远挂起。 */
export function registerDialogHost(): () => void {
  hostCount += 1
  return () => {
    hostCount -= 1
    if (hostCount > 0) return
    const pending = queue
    queue = []
    emit()
    for (const request of pending) request.resolve(false)
  }
}
