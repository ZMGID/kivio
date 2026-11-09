import { useState } from 'react'
import type { McpServerIcon as IconImage } from '../api/tauri'
import { McpIcon } from '../settings/public/icons'

function IconContent({ src }: { src?: string }) {
  const [failed, setFailed] = useState(false)
  if (!src?.startsWith('data:image/png;base64,') || failed) return <McpIcon size={24} />
  return <img src={src} alt="" aria-hidden="true" width={32} height={32}
    className="h-8 w-8 object-contain" onError={() => setFailed(true)} />
}

export function McpServerIcon({ icons = [] }: { icons?: IconImage[] }) {
  const neutral = icons.find(icon => !icon.theme) ?? icons[0]
  const light = icons.find(icon => icon.theme === 'light') ?? neutral
  const dark = icons.find(icon => icon.theme === 'dark') ?? neutral
  return <span aria-hidden="true" className="flex h-8 w-8 shrink-0 items-center justify-center text-neutral-500">
    <span className="flex items-center justify-center dark:hidden"><IconContent key={light?.src} src={light?.src} /></span>
    <span className="hidden items-center justify-center dark:flex"><IconContent key={dark?.src} src={dark?.src} /></span>
  </span>
}
