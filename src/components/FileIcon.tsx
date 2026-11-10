import { getBuiltInSpriteSheet } from '@pierre/trees'
import { Folder as ClosedFolder, FolderOpen as OpenFolder } from 'lucide-react'
import { useInsertionEffect, type CSSProperties } from 'react'
import { fileIconColors, fileIconResolver, fileKindVisual, folderKindVisual } from './fileIconVisual'

function ensureFileIconSprite() {
  if (document.getElementById('kivio-file-icon-sprite')) return
  const sprite = document.createElement('div')
  sprite.id = 'kivio-file-icon-sprite'
  sprite.style.cssText = 'position:absolute;width:0;height:0;overflow:hidden;pointer-events:none'
  sprite.setAttribute('aria-hidden', 'true')
  // Only the pinned package's built-in SVG enters the document, never file names or content.
  sprite.innerHTML = getBuiltInSpriteSheet('complete')
  document.body.prepend(sprite)
}

export function FolderIcon({ open = false, variant = 'folder', size = 16, className = '', style }: {
  open?: boolean; variant?: 'folder' | 'tree'; size?: number | string; className?: string; style?: CSSProperties
}) {
  useInsertionEffect(ensureFileIconSprite, [])
  if (variant === 'tree') {
    return <svg aria-hidden="true" focusable="false" width={size} height={size} viewBox="0 0 16 16"
      className={`${folderKindVisual().iconClass} ${className}`} style={{ ...style, transform: open ? undefined : 'rotate(-90deg)' }}>
      <use href="#file-tree-icon-chevron" />
    </svg>
  }
  const Glyph = open ? OpenFolder : ClosedFolder
  return <Glyph aria-hidden="true" focusable="false" size={size} strokeWidth={2}
    className={`${folderKindVisual().iconClass} ${className}`} style={style} />
}

export function FileIcon({ name, folder = false, size = 16, className = '' }: {
  name: string; folder?: boolean; size?: number | string; className?: string
}) {
  useInsertionEffect(ensureFileIconSprite, [])
  const visual = folder ? folderKindVisual() : fileKindVisual(name)
  if (folder) return <FolderIcon size={size} className={className} />
  const icon = fileIconResolver.resolveIcon('file-tree-icon-file', name.replace(/\\/g, '/'))
  return (
    <svg aria-hidden="true" focusable="false" width={size} height={size} viewBox="0 0 16 16"
      className={`${className} ${fileIconColors.get(icon.token ?? '') ?? visual.iconClass}`}>
      <use href={`#${icon.name}`} />
    </svg>
  )
}
