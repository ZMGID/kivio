import { FolderIcon as Folder } from '../components/FileIcon'
import { useEffect, useState } from 'react'

import { dockApi } from './dock/api'

// Rows mounting together share the read, and a resolved logo is reused briefly so switching
// tabs or re-rendering a long list does not re-read every project; after the window a
// remount can still discover a changed logo.
const ICON_REUSE_MS = 60_000
const pendingIcons = new Map<string, Promise<string | null>>()
const resolvedIcons = new Map<string, { url: string | null; at: number }>()

function readProjectIcon(workdir: string): Promise<string | null> {
  const cached = resolvedIcons.get(workdir)
  if (cached && Date.now() - cached.at < ICON_REUSE_MS) return Promise.resolve(cached.url)
  let request = pendingIcons.get(workdir)
  if (!request) {
    request = dockApi.projectIcon(workdir).catch(() => null)
    pendingIcons.set(workdir, request)
    void request.then(url => {
      resolvedIcons.set(workdir, { url, at: Date.now() })
      pendingIcons.delete(workdir)
    })
  }
  return request
}

export function ProjectIcon({ workdir, color, name }: { workdir?: string | null; color?: string | null; name?: string | null }) {
  const [loaded, setLoaded] = useState<{ workdir: string; url: string } | null>(() => {
    const cached = workdir ? resolvedIcons.get(workdir) : undefined
    return workdir && cached?.url && Date.now() - cached.at < ICON_REUSE_MS ? { workdir, url: cached.url } : null
  })
  useEffect(() => {
    if (!workdir) return
    let current = true
    void readProjectIcon(workdir).then(url => {
      if (current) setLoaded(url ? { workdir, url } : null)
    })
    return () => { current = false }
  }, [workdir])

  return loaded?.workdir === workdir && loaded?.url
    ? <img src={loaded.url} alt="" className="h-3.5 w-3.5 shrink-0 object-contain" onError={() => setLoaded(null)} />
    : name?.trim()
      ? <ProjectInitial name={name} color={color} />
      : <Folder size={14} className="shrink-0" style={color ? { color } : undefined} aria-hidden="true" />
}

// 与集/助手同一组色；未设颜色的项目按名称稳定取色，同名始终同色。
const initialColors = ['#6A8FBD', '#2f6ff0', '#4F9D7A', '#8A6FBD', '#B7791F', '#5E8C6A', '#C2566B', '#3E8E9E']

function colorForName(name: string): string {
  let hash = 0
  for (const ch of name) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0
  return initialColors[hash % initialColors.length]
}

/** 没有 logo 的项目：彩色小方块 + 名称首字（大写），列表里一眼能分清。 */
function ProjectInitial({ name, color }: { name: string; color?: string | null }) {
  const initial = (Array.from(name.trim())[0] ?? '').toUpperCase()
  return (
    <span
      className="grid size-3.5 shrink-0 place-items-center rounded-[4px] text-[9px] font-semibold leading-none text-white"
      style={{ backgroundColor: color || colorForName(name.trim()) }}
      aria-hidden="true"
    >
      {initial}
    </span>
  )
}
