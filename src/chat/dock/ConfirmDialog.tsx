// Dock 内共享的确认对话框（删除文件 / 丢弃变更等破坏性操作二次确认）。
// 样式沿用 Chat.tsx pendingToolConfirm 的固定浮层模式。
import { TriangleAlert } from 'lucide-react'
import { i18n, type Lang } from '../../components/i18n'
import { Button } from '../../components/Button'

type ConfirmDialogProps = {
  lang: Lang
  title: string
  message: string
  confirmLabel: string
  busy?: boolean
  onConfirm: () => void
  onCancel: () => void
}

export function ConfirmDialog({
  lang,
  title,
  message,
  confirmLabel,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const t = i18n[lang]
  return (
    <div className="fixed inset-0 z-[300] flex items-center justify-center bg-neutral-900/20 px-4">
      <div className="w-full max-w-sm rounded-lg border border-neutral-200 bg-neutral-50 p-4 shadow-xl">
        <div className="mb-3 flex items-start gap-2">
          <TriangleAlert size={17} className="mt-0.5 shrink-0 text-danger" />
          <div className="min-w-0 flex-1">
            <div className="text-[14px] font-semibold text-neutral-900">{title}</div>
            <div className="mt-1 break-all text-[12px] text-neutral-500">{message}</div>
          </div>
        </div>
        <div className="flex justify-end gap-2">
          <Button onClick={onCancel} disabled={busy}>
            {t.cancel}
          </Button>
          <Button variant="danger" onClick={onConfirm} disabled={busy}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  )
}
