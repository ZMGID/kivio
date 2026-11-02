import { useEffect, useMemo, useRef, useState, type ReactNode, type Ref } from 'react'
import { Code2, ExternalLink, Maximize2, X } from 'lucide-react'
import { api } from '../api/tauri'
import { IconButton } from '../components/Button'
import { notifyError } from '../components/appNoticeStore'
import { ChatHeavyIsland } from './ChatHeavyIsland'
import { readThemeToken, useThemeKey } from './chatTheme'

const MESSAGE = 'kivioHtmlPreview'
const clampHeight = (height: number) => Math.min(2000, Math.max(80, Math.ceil(height)))

function htmlThemeCss(): string {
  const dark = document.documentElement.classList.contains('dark')
  const token = (name: string, light: string, darkValue: string) => readThemeToken(name, dark ? darkValue : light)
  const surface = token('--theme-surface', '#fdfcfa', '#212121')
  const soft = token('--theme-surface-soft', '#f9f8f6', '#262629')
  const muted = token('--theme-surface-muted', '#f4f3f1', '#2a2a2d')
  const border = token('--theme-surface-border', '#e2e1df', '#3a3a40')
  const input = token('--theme-surface-border-strong', '#d2d1cf', '#4e4e56')
  const text = token('--text', '#1d1d1f', '#f0f0f3')
  const mutedText = token('--text-muted', '#737373', '#a1a1aa')
  const accent = token('--accent', '#2f6ff0', '#5c8df7')
  const accentSoft = token('--accent-soft', '#e6efff', '#2a3243')
  const onAccent = token('--text-onaccent', '#ffffff', '#ffffff')
  const danger = token('--danger', '#c4341c', '#ef6a5a')
  const warning = token('--warn', '#c47b16', '#e6a04a')
  const success = token('--success', '#10b981', '#10b981')
  const info = '#3b82f6'
  const charts = dark
    ? ['#2dd4bf', '#fbbf24', '#c084fc', '#fb7185', '#a3e635']
    : ['#0d9488', '#d97706', '#9333ea', '#e11d48', '#65a30d']
  const sans = readThemeToken('--font-sans', 'system-ui,-apple-system,"Segoe UI",sans-serif')
  const mono = readThemeToken('--font-mono', 'ui-monospace,Menlo,Consolas,monospace')
  return `:root{color-scheme:${dark ? 'dark' : 'light'};`
    + `--background:${surface};--foreground:${text};--muted:${muted};--muted-foreground:${mutedText};`
    + `--card:${soft};--card-foreground:${text};--popover:${surface};--popover-foreground:${text};`
    + `--secondary:${muted};--secondary-foreground:${text};--border:${border};--input:${input};--ring:${accent};`
    + `--primary:${accent};--primary-foreground:${onAccent};--accent:${accent};--accent-foreground:${onAccent};`
    + `--destructive:${danger};--destructive-foreground:${onAccent};--warning:${warning};--warning-foreground:${dark ? '#fbbf24' : '#92400e'};`
    + `--success:${success};--success-foreground:${dark ? '#34d399' : '#047857'};--info:${info};--info-foreground:${dark ? '#60a5fa' : '#1d4ed8'};`
    + `--code-background:${muted};--code-foreground:${text};--chart-1:${accent};`
    + charts.map((color, index) => `--chart-${index + 2}:${color};`).join('')
    + `--radius:0.625rem;--font-sans:${sans};--font-mono:${mono}}`
    + `html,body{margin:0;background:transparent}body{color:var(--foreground);font:14px/1.6 var(--font-sans);overflow-wrap:anywhere}a{color:var(--accent)}::selection{background:${accentSoft}}`
}

function themedFrameUrl(src: string): string {
  let binary = ''
  for (const byte of new TextEncoder().encode(htmlThemeCss())) binary += String.fromCharCode(byte)
  const encoded = btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
  return `${src.split('#', 1)[0]}#kivio-theme=${encoded}`
}

function postTheme(target: Window | null | undefined) {
  target?.postMessage({ [MESSAGE]: 'theme', value: htmlThemeCss() }, '*')
}

export function HtmlFrame({
  src, title = 'HTML 预览', initialHeight = 160, onSource, onOpenInBrowser, fullscreenHtml, failed = false, fallback = null,
}: {
  src: string | null
  title?: string
  initialHeight?: number
  onSource?: () => void
  onOpenInBrowser: (themeCss: string) => void
  fullscreenHtml?: string
  failed?: boolean
  fallback?: ReactNode
}) {
  const [fullscreen, setFullscreen] = useState(false)
  const [height, setHeight] = useState(() => clampHeight(initialHeight))
  const [measured, setMeasured] = useState(false)
  const [loadFailed, setLoadFailed] = useState(false)
  const [dialogPreviewUrl, setDialogPreviewUrl] = useState<string | null>(null)
  const [dialogFailed, setDialogFailed] = useState(false)
  const frameRef = useRef<HTMLIFrameElement>(null)
  const dialogFrameRef = useRef<HTMLIFrameElement>(null)
  const dialogRef = useRef<HTMLDialogElement>(null)
  const theme = useThemeKey()
  const inlineSrc = useMemo(() => src ? themedFrameUrl(src) : null, [src])
  const dialogSrc = fullscreenHtml === undefined ? src : dialogPreviewUrl
  const fullscreenSrc = useMemo(() => fullscreen && dialogSrc ? themedFrameUrl(dialogSrc) : null, [fullscreen, dialogSrc])

  useEffect(() => {
    setHeight(clampHeight(initialHeight))
    setMeasured(false)
    setLoadFailed(false)
  }, [src, initialHeight])

  useEffect(() => {
    postTheme(frameRef.current?.contentWindow)
    postTheme(dialogFrameRef.current?.contentWindow)
  }, [theme])

  useEffect(() => {
    if (!fullscreen || fullscreenHtml === undefined) return
    let active = true
    let id: string | null = null
    void api.createHtmlPreview(fullscreenHtml).then((preview) => {
      id = preview.id
      if (active) setDialogPreviewUrl(preview.url)
      else void api.releaseHtmlPreview(preview.id).catch(() => {})
    }).catch(() => {
      if (active) setDialogFailed(true)
    })
    return () => {
      active = false
      if (id) void api.releaseHtmlPreview(id).catch(() => {})
    }
  }, [fullscreen, fullscreenHtml])

  useEffect(() => {
    if (!src) return
    const onMessage = (event: MessageEvent) => {
      const data = event.data as Record<string, unknown> | null
      if (!data || typeof data !== 'object') return
      const inline = frameRef.current?.contentWindow
      const dialog = dialogFrameRef.current?.contentWindow
      const fromInline = !!inline && event.source === inline
      if (!fromInline && !(fullscreen && dialog && event.source === dialog)) return
      if (fromInline && data[MESSAGE] === 'height' && typeof data.value === 'number' && Number.isFinite(data.value)) {
        setHeight(clampHeight(data.value))
        setMeasured(true)
      } else if (data[MESSAGE] === 'error') {
        if (fromInline) setLoadFailed(true)
        else setDialogFailed(true)
      } else if (data[MESSAGE] === 'open' && typeof data.value === 'string'
        && /^https?:\/\//i.test(data.value) && navigator.userActivation?.isActive) {
        void api.openExternal(data.value).catch((error) => notifyError(`打开链接失败：${String(error)}`))
      }
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [src, fullscreen])

  useEffect(() => {
    const dialog = dialogRef.current
    if (fullscreen && src && !failed && !loadFailed && dialog && !dialog.open) dialog.showModal()
  }, [fullscreen, src, failed, loadFailed])

  if (failed || loadFailed) return fallback

  const frame = (ref: Ref<HTMLIFrameElement>, frameSrc: string, className: string, isDialog: boolean, frameHeight?: number) => (
    <iframe
      ref={ref}
      title={title}
      src={frameSrc}
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      className={className}
      style={frameHeight === undefined ? undefined : { height: frameHeight }}
      onLoad={(event) => postTheme(event.currentTarget.contentWindow)}
      onError={() => isDialog ? setDialogFailed(true) : setLoadFailed(true)}
    />
  )

  return (
    <ChatHeavyIsland minHeight={height} fallback={<div className="my-3" style={{ height }} />} eager>
      <div className="group/html not-prose relative my-3" data-chat-async-pending={measured ? undefined : 'true'}>
        {inlineSrc ? frame(frameRef, inlineSrc, 'block w-full border-0 bg-transparent', false, height) : <div style={{ height }} />}
        <div className="absolute right-0 top-0 flex gap-0.5 rounded-md bg-[var(--theme-surface)] opacity-0 shadow-sm transition-opacity focus-within:opacity-100 group-hover/html:opacity-100">
          <IconButton size="sm" onClick={() => { setDialogPreviewUrl(null); setDialogFailed(false); setFullscreen(true) }} label="全屏查看" disabled={!src}>
            <Maximize2 size={14} strokeWidth={2} />
          </IconButton>
          {onSource && <IconButton size="sm" onClick={onSource} label="查看源码">
            <Code2 size={14} strokeWidth={2} />
          </IconButton>}
          <IconButton size="sm" onClick={() => onOpenInBrowser(htmlThemeCss())} label="在浏览器打开">
            <ExternalLink size={14} strokeWidth={2} />
          </IconButton>
        </div>
      </div>
      {fullscreen && src ? (
        <dialog ref={dialogRef} className="kv-modal kv-html-preview-full" aria-label={title}
          onClose={() => setFullscreen(false)}
          onClick={(event) => { if (event.target === event.currentTarget) dialogRef.current?.close() }}>
          <div className="flex justify-end">
            <IconButton size="sm" onClick={() => dialogRef.current?.close()} label="关闭">
              <X size={14} strokeWidth={2} />
            </IconButton>
          </div>
          {dialogFailed ? (
            <div role="status" className="p-4 text-[13px] text-[var(--text-muted)]">全屏页面不可用</div>
          ) : fullscreenSrc ? frame(dialogFrameRef, fullscreenSrc, 'min-h-0 w-full flex-1 border-0 bg-transparent', true)
            : <div role="status" className="p-4 text-[13px] text-[var(--text-muted)]">正在加载页面…</div>}
        </dialog>
      ) : null}
    </ChatHeavyIsland>
  )
}
