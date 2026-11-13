import { FolderIcon as FolderOpen } from '../components/FileIcon'
import { useT } from '../components/i18n'

import { useState } from 'react'
import { FileIcon } from '../components/FileIcon'
import { fileKindVisual, folderKindVisual } from '../components/fileIconVisual'
import { DockContextMenu, type DockMenuAnchor } from './dock/DockContextMenu'

/** 正文里本地文件链接前的类型图标；与附件卡片共用同一份类型映射。 */
export function FileKindIcon({ name, folder = false }: { name: string; folder?: boolean }) {
  // U+2060 禁止图标与后面的文字之间换行，否则长路径会把图标单独留在上一行。
  return <><FileIcon name={name} folder={folder} size="1em" className="mr-1 inline-block align-[-0.125em]" />{'\u2060'}</>
}

/** 上传附件使用卡片，正文产物使用跟随文字行高的链接。 */
export function FileChip({
  name,
  kind,
  onClick,
  ariaLabel,
  variant = 'card',
  onRevealLocation,
}: {
  name: string
  kind?: 'folder'
  onClick: () => void
  ariaLabel?: string
  variant?: 'card' | 'inline' | 'compact'
  onRevealLocation?: () => Promise<void>
}) {
  const t = useT()
  const [menuAnchor, setMenuAnchor] = useState<DockMenuAnchor | null>(null)
  const [locationError, setLocationError] = useState(false)
  const visual = kind === 'folder' ? folderKindVisual() : fileKindVisual(name)
  const contextMenu = onRevealLocation ? (event: React.MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()
    setLocationError(false)
    setMenuAnchor({ left: event.clientX, top: event.clientY })
  } : undefined
  const menu = <>
    {menuAnchor && onRevealLocation && <DockContextMenu anchor={menuAnchor} onClose={() => setMenuAnchor(null)} items={[{
      key: 'reveal', label: t.chatMsgRevealLocation, icon: <FolderOpen open size={16} />,
      onSelect: () => { void onRevealLocation().catch(() => setLocationError(true)) },
    }]} />}
    {locationError && <span role="status" className="ml-1 text-xs text-neutral-500">{t.chatMsgRevealLocationFailed}</span>}
  </>
  if (variant === 'inline') {
    return (
      <>
      <button
        type="button"
        onClick={onClick}
        onContextMenu={contextMenu}
        className="inline max-w-full cursor-pointer rounded-sm border-0 bg-transparent p-0 text-left align-baseline text-blue-600 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] dark:text-blue-400"
        style={{ font: 'inherit', overflowWrap: 'anywhere' }}
        title={name}
        aria-label={ariaLabel ?? name}
      >
        <FileKindIcon name={name} folder={kind === 'folder'} />
        {name}
      </button>
      {menu}
      </>
    )
  }
  return (
    <>
    <button
      type="button"
      onClick={onClick}
      onContextMenu={contextMenu}
      className={`flex shrink-0 items-center gap-2 rounded-lg border border-neutral-200/90 bg-neutral-50 text-left hover:bg-neutral-100 ${variant === 'compact' ? 'h-8 w-[18rem] max-w-full px-2' : 'h-16 w-[9.5rem] px-1.5'}`}
      title={name}
      aria-label={ariaLabel ?? name}
    >
      <span className={`flex shrink-0 items-center justify-center rounded-md ${variant === 'compact' ? 'h-5 w-5' : `h-11 w-11 ${visual.wellClass}`}`}>
        <FileIcon name={name} folder={kind === 'folder'} size={variant === 'compact' ? 14 : 18} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[12px] font-medium leading-tight text-neutral-800">
          {name}
        </span>
        {variant !== 'compact' && <span className="mt-0.5 block truncate text-[10px] font-medium uppercase tracking-wide text-neutral-400 dark:text-neutral-500">
          {visual.label}
        </span>}
      </span>
    </button>
    {menu}
    </>
  )
}
