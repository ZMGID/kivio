import { useEffect, useId, useRef, useSyncExternalStore } from 'react'
import { Button } from './Button'
import { useT } from './i18n'
import { currentDialog, registerDialogHost, settleDialog, subscribeDialogs, type DialogRequest } from './dialogQueue'

// 队列状态与 confirmDialog / alertDialog 见 ./dialogQueue.ts。

/** 每个窗口根部挂一个；同一时间只显示队首的一个对话框。 */
export function AppDialogHost() {
  useEffect(() => registerDialogHost(), [])
  const request = useSyncExternalStore(subscribeDialogs, currentDialog, currentDialog)
  return request ? <AppDialog key={request.id} request={request} /> : null
}

function AppDialog({ request }: { request: DialogRequest }) {
  const t = useT()
  const dialogRef = useRef<HTMLDialogElement>(null)
  const confirmRef = useRef<HTMLButtonElement>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const titleId = useId()
  const messageId = useId()
  const isConfirm = request.kind === 'confirm'
  const danger = request.kind === 'confirm' && Boolean(request.options.danger)
  const { title, message } = request.options

  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) return
    // 记下原焦点，关闭后还回去（原生 <dialog> 在元素被卸载时不会自动还焦点）。
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    if (!dialog.open) dialog.showModal()
    const initialFocus = danger ? cancelRef.current : confirmRef.current
    initialFocus?.focus()
    return () => {
      dialog.close()
      if (previous?.isConnected) previous.focus()
    }
  }, [danger])

  const finish = (value: boolean) => settleDialog(request.id, value)

  const confirmLabel = request.kind === 'confirm'
    ? request.options.confirmLabel || t.dialogConfirm
    : request.options.okLabel || t.dialogOk
  const cancelLabel = request.kind === 'confirm' ? request.options.cancelLabel || t.cancel : ''

  return (
    <dialog
      ref={dialogRef}
      className="kv-modal kv-app-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby={title ? titleId : messageId}
      aria-describedby={title ? messageId : undefined}
      // 模态框：按键不再冒泡到 window 上的监听（图片查看器 / 菜单的 Esc 等），免得连带关掉下层。
      onKeyDown={(event) => event.stopPropagation()}
      onCancel={(event) => {
        // Esc：不让浏览器自行关掉 <dialog>，统一走 finish 以便答复 Promise。
        event.preventDefault()
        finish(false)
      }}
    >
      {title && <h2 id={titleId} className="kv-app-dialog-title">{title}</h2>}
      <p id={messageId} className="kv-app-dialog-message custom-scrollbar">{message}</p>
      <div className="kv-app-dialog-actions">
        {isConfirm && (
          <Button ref={cancelRef} onClick={() => finish(false)}>
            {cancelLabel}
          </Button>
        )}
        <Button
          ref={confirmRef}
          variant={danger ? 'danger' : 'primary'}
          onClick={() => finish(true)}
        >
          {confirmLabel}
        </Button>
      </div>
    </dialog>
  )
}
