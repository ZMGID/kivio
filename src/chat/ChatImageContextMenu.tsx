import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { save } from '@tauri-apps/plugin-dialog'
import { Check, Clipboard, Download, FolderOpen, Maximize2 } from 'lucide-react'
import { api } from '../api/tauri'
import { useT } from '../components/i18n'
import { alertDialog } from '../components/dialogQueue'
import { useCloseAnimation } from './useCloseAnimation'
import { useClampedMenuPosition } from './useClampedMenuPosition'
import { base64FromDataUrl, imageExtension } from './imageData'

export interface ChatImageMenuAnchor {
  left: number
  top: number
}

interface ChatImageContextMenuProps {
  anchor: ChatImageMenuAnchor
  /** 图片的 data URL（复制/另存都从它取字节）。 */
  src: string
  /** 列表显示缩略图时，复制/另存前按需取原图。 */
  loadFullSrc?: () => Promise<string | null>
  name?: string
  onOpenViewer?: () => void
  onRevealLocation?: () => Promise<void>
  onClose: () => void
}

export function ChatImageContextMenu({
  anchor,
  src,
  loadFullSrc,
  name,
  onOpenViewer,
  onRevealLocation,
  onClose: onCloseProp,
}: ChatImageContextMenuProps) {
  const t = useT()
  const menuRef = useRef<HTMLDivElement>(null)
  const pos = useClampedMenuPosition(menuRef, anchor)
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onCloseProp)
  const onClose = startClose
  const [copied, setCopied] = useState(false)
  const [fullImageError, setFullImageError] = useState(false)
  const [locationError, setLocationError] = useState(false)
  const base64 = base64FromDataUrl(src)

  const actionImage = async (): Promise<string | null> => {
    if (!loadFullSrc) return src
    const full = await loadFullSrc()
    setFullImageError(!full)
    return full
  }

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return
      onClose()
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', onPointerDown)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('mousedown', onPointerDown)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [onClose])

  const handleCopy = async () => {
    const image = await actionImage()
    if (!image) return
    const payload = base64FromDataUrl(image)
    if (!payload) return
    // 复用 Lens 标注早就有的剪贴板写图命令（解码 → arboard set_image），不另造一条。
    const result = await api.lensCopyImageToClipboard(payload)
    if (!result.success) {
      void alertDialog(t.chatImageCopyFailed.replace('{error}', () => result.error ?? t.chatUnknownError))
      return
    }
    setCopied(true)
    window.setTimeout(onClose, 600)
  }

  const handleSave = async () => {
    const image = await actionImage()
    if (!image) return
    const payload = base64FromDataUrl(image)
    if (!payload) return
    const ext = imageExtension(image, name)
    const path = await save({
      defaultPath: name || `image.${ext}`,
      filters: [{ name: 'Image', extensions: [ext] }],
    })
    if (!path) return
    const result = await api.lensSaveAnnotatedPng(payload, path)
    if (!result.success) void alertDialog(t.chatImageSaveFailed.replace('{error}', () => result.error ?? t.chatUnknownError))
    onClose()
  }

  const itemClass =
    'kv-menu-item'
  const iconClass = ''

  const menu = (
    <div
      ref={menuRef}
      className={`kv-menu ${closing ? 'chat-motion-popover-out' : 'chat-motion-popover chat-motion-menu-cascade'} fixed z-[200] min-w-[176px]`}
      style={{ left: pos.left, top: pos.top }}
      role="menu"
      onAnimationEnd={onAnimationEnd}
    >
      <button
        type="button"
        role="menuitem"
        className={itemClass}
        disabled={!base64}
        onClick={() => void handleCopy()}
      >
        {copied ? (
          <Check size={16} strokeWidth={2} className="shrink-0 text-neutral-500 chat-motion-pop" />
        ) : (
          <Clipboard size={16} strokeWidth={1.75} className={iconClass} />
        )}
        {copied ? t.chatImageCopied : t.chatImageCopy}
      </button>
      <button
        type="button"
        role="menuitem"
        className={itemClass}
        disabled={!base64}
        onClick={() => void handleSave()}
      >
        <Download size={16} strokeWidth={1.75} className={iconClass} />
        {t.chatImageSaveAs}
      </button>
      {onOpenViewer ? (
        <button
          type="button"
          role="menuitem"
          className={itemClass}
          onClick={() => {
            onOpenViewer()
            onClose()
          }}
        >
          <Maximize2 size={16} strokeWidth={1.75} className={iconClass} />
          {t.chatImageViewLarge}
        </button>
      ) : null}
      {onRevealLocation && <button type="button" role="menuitem" className={itemClass} onClick={() => {
        setLocationError(false)
        void onRevealLocation().then(onClose).catch(() => setLocationError(true))
      }}>
        <FolderOpen size={16} strokeWidth={1.75} />
        {t.chatImageRevealLocation}
      </button>}
      {locationError && <span role="status" className="block max-w-64 px-3 py-1 text-xs text-neutral-500">{t.chatImageRevealFailed}</span>}
      {fullImageError && <span role="status" className="block max-w-64 px-3 py-1 text-xs text-neutral-500">{t.chatImageLoadOriginalFailed}</span>}
    </div>
  )

  return createPortal(menu, document.body)
}
