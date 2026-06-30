import { useEffect, useRef, type RefObject } from 'react'
import type { LensReplaceLine } from '../api/tauri'
import type { CapturedFrame } from './types'

type ReplaceTranslateOverlayProps = {
  frame: CapturedFrame
  imagePreview: string
  lines: LensReplaceLine[]
  phase: 'ocr' | 'translating' | 'done' | ''
  error?: string
  statusLabel: string
  escHint: string
  freezeCanvasRef: RefObject<HTMLCanvasElement | null>
}

function sampleAverageColorFromCtx(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  maxW: number,
  maxH: number,
): string {
  try {
    const sx = Math.max(0, Math.min(maxW - 1, Math.floor(x)))
    const sy = Math.max(0, Math.min(maxH - 1, Math.floor(y)))
    const sw = Math.max(1, Math.min(maxW - sx, Math.floor(w)))
    const sh = Math.max(1, Math.min(maxH - sy, Math.floor(h)))
    const data = ctx.getImageData(sx, sy, sw, sh).data
    let r = 0
    let g = 0
    let b = 0
    let count = 0
    for (let i = 0; i < data.length; i += 16) {
      r += data[i]
      g += data[i + 1]
      b += data[i + 2]
      count += 1
    }
    if (count === 0) return 'rgb(255,255,255)'
    return `rgb(${Math.round(r / count)},${Math.round(g / count)},${Math.round(b / count)})`
  } catch {
    return 'rgb(255,255,255)'
  }
}

function sampleFromFreezeCanvas(
  freezeCanvas: HTMLCanvasElement,
  viewportW: number,
  viewportH: number,
  globalX: number,
  globalY: number,
  w: number,
  h: number,
): string | null {
  if (viewportW <= 0 || viewportH <= 0 || freezeCanvas.width <= 0 || freezeCanvas.height <= 0) {
    return null
  }
  const ctx = freezeCanvas.getContext('2d')
  if (!ctx) return null
  const scaleX = freezeCanvas.width / viewportW
  const scaleY = freezeCanvas.height / viewportH
  return sampleAverageColorFromCtx(
    ctx,
    globalX * scaleX,
    globalY * scaleY,
    w * scaleX,
    h * scaleY,
    freezeCanvas.width,
    freezeCanvas.height,
  )
}

function luminance(r: number, g: number, b: number): number {
  return 0.299 * r + 0.587 * g + 0.114 * b
}

function parseRgb(color: string): [number, number, number] {
  const m = color.match(/rgb\((\d+),\s*(\d+),\s*(\d+)\)/)
  if (!m) return [255, 255, 255]
  return [Number(m[1]), Number(m[2]), Number(m[3])]
}

function wrapTextLines(
  ctx: CanvasRenderingContext2D,
  text: string,
  maxWidth: number,
  fontSize: number,
): string[] {
  ctx.font = `${fontSize}px system-ui, "Segoe UI", sans-serif`
  const lines: string[] = []
  let current = ''

  const pushToken = (token: string) => {
    if (!token) return
    const candidate = current ? `${current}${token}` : token
    if (ctx.measureText(candidate).width <= maxWidth || !current) {
      current = candidate
      return
    }
    if (current) lines.push(current)
    current = token
    if (ctx.measureText(current).width > maxWidth) {
      for (const ch of token) {
        const next = current + ch
        if (ctx.measureText(next).width <= maxWidth || !current) {
          current = next
        } else {
          lines.push(current)
          current = ch
        }
      }
    }
  }

  for (const word of text.split(/(\s+)/)) {
    pushToken(word)
  }
  if (current) lines.push(current)
  return lines.length > 0 ? lines : ['']
}

function drawTextInBox(
  ctx: CanvasRenderingContext2D,
  text: string,
  bx: number,
  by: number,
  bw: number,
  bh: number,
  fillColor: string,
) {
  const pad = 2
  const innerW = Math.max(1, bw - pad * 2)
  const innerH = Math.max(1, bh - pad * 2)
  let fontSize = Math.min(Math.max(10, innerH * 0.72), 18)

  let lines: string[] = []
  let lineHeight = fontSize * 1.15
  while (fontSize >= 8) {
    lines = wrapTextLines(ctx, text, innerW, fontSize)
    lineHeight = fontSize * 1.15
    if (lines.length * lineHeight <= innerH) break
    fontSize -= 1
  }

  ctx.font = `${fontSize}px system-ui, "Segoe UI", sans-serif`
  ctx.fillStyle = fillColor
  ctx.textBaseline = 'top'
  ctx.textAlign = 'left'

  const totalHeight = lines.length * lineHeight
  let y = by + pad + Math.max(0, (innerH - totalHeight) / 2)
  for (const line of lines) {
    ctx.fillText(line.trim(), bx + pad, y, innerW)
    y += lineHeight
  }
}

export function ReplaceTranslateOverlay({
  frame,
  imagePreview,
  lines,
  phase,
  error,
  statusLabel,
  escHint,
  freezeCanvasRef,
}: ReplaceTranslateOverlayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const fallbackCanvasRef = useRef<HTMLCanvasElement | null>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !imagePreview || lines.length === 0) return

    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const img = new Image()
    img.onload = () => {
      const w = Math.max(1, Math.round(frame.width))
      const h = Math.max(1, Math.round(frame.height))
      canvas.width = w
      canvas.height = h
      ctx.clearRect(0, 0, w, h)

      const scaleX = w / img.naturalWidth
      const scaleY = h / img.naturalHeight
      const freezeCanvas = freezeCanvasRef.current
      const viewportW = window.innerWidth
      const viewportH = window.innerHeight

      let fallbackCtx: CanvasRenderingContext2D | null = null
      if (!freezeCanvas || freezeCanvas.width <= 0) {
        if (!fallbackCanvasRef.current) {
          fallbackCanvasRef.current = document.createElement('canvas')
        }
        const fb = fallbackCanvasRef.current
        fb.width = img.naturalWidth
        fb.height = img.naturalHeight
        fallbackCtx = fb.getContext('2d')
        fallbackCtx?.drawImage(img, 0, 0)
      }

      for (const line of lines) {
        const displayText = line.translated.trim() || line.text
        if (!displayText) continue

        const bx = line.x * scaleX
        const by = line.y * scaleY
        const bw = Math.max(1, line.width * scaleX)
        const bh = Math.max(1, line.height * scaleY)

        let bg: string | null = null
        if (freezeCanvas && freezeCanvas.width > 0) {
          bg = sampleFromFreezeCanvas(
            freezeCanvas,
            viewportW,
            viewportH,
            frame.x + bx,
            frame.y + by,
            bw,
            bh,
          )
        }
        if (!bg && fallbackCtx) {
          bg = sampleAverageColorFromCtx(
            fallbackCtx,
            line.x,
            line.y,
            line.width,
            line.height,
            img.naturalWidth,
            img.naturalHeight,
          )
        }
        bg ??= 'rgb(255,255,255)'

        ctx.fillStyle = bg
        ctx.fillRect(bx, by, bw, bh)

        const [r, g, b] = parseRgb(bg)
        drawTextInBox(ctx, displayText, bx, by, bw, bh, luminance(r, g, b) > 140 ? '#111827' : '#f9fafb')
      }
    }
    img.src = imagePreview.startsWith('data:') ? imagePreview : `data:image/png;base64,${imagePreview}`
  }, [frame.height, frame.width, frame.x, frame.y, freezeCanvasRef, imagePreview, lines])

  const showOverlay = lines.length > 0 && (phase === 'ocr' || phase === 'translating' || phase === 'done')

  return (
    <>
      <div className="absolute top-3 left-1/2 -translate-x-1/2 z-30 pointer-events-none">
        <div className="px-3 py-1.5 rounded-full text-[12px] font-medium bg-black/70 text-white shadow-lg backdrop-blur-sm">
          {error || statusLabel}
          {!error && phase !== 'done' && (
            <span className="ml-2 inline-block w-1.5 h-1.5 rounded-full bg-white animate-pulse align-middle" />
          )}
        </div>
        <p className="text-center text-[11px] text-white/80 mt-1 drop-shadow">{escHint}</p>
      </div>
      {showOverlay && (
        <div
          className="absolute z-20 pointer-events-none"
          style={{
            left: frame.x,
            top: frame.y,
            width: frame.width,
            height: frame.height,
          }}
        >
          <canvas
            ref={canvasRef}
            className="block"
            style={{ width: frame.width, height: frame.height }}
          />
        </div>
      )}
    </>
  )
}
