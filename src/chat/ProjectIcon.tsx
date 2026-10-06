import { useEffect, useState } from 'react'
import { Folder } from 'lucide-react'
import { dockApi } from './dock/api'

// Rows mounting together share the read; remounting can discover a changed logo.
const pendingIcons = new Map<string, Promise<string | null>>()

export function ProjectIcon({ workdir, color }: { workdir?: string | null; color?: string | null }) {
  const [loaded, setLoaded] = useState<{ workdir: string; url: string } | null>(null)
  useEffect(() => {
    if (!workdir) return
    let current = true
    let request = pendingIcons.get(workdir)
    if (!request) {
      request = dockApi.projectIcon(workdir).catch(() => null)
      pendingIcons.set(workdir, request)
      void request.finally(() => pendingIcons.delete(workdir))
    }
    void request.then(url => {
      if (current) setLoaded(url ? { workdir, url } : null)
    })
    return () => { current = false }
  }, [workdir])

  return loaded?.workdir === workdir && loaded?.url
    ? <img src={loaded.url} alt="" className="h-3.5 w-3.5 shrink-0 object-contain" onError={() => setLoaded(null)} />
    : <Folder size={14} className="shrink-0" style={color ? { color } : undefined} aria-hidden="true" />
}
