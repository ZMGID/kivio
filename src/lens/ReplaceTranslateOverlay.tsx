import { useEffect, useRef } from 'react'
import type { LensReplaceGroup, LensReplaceRenderSlot } from '../api/tauri'
import { layoutReplaceTextFlow, replaceTextVerticalOffset, type ReplaceTextFlowSlotLayout } from './replaceTextLayout'
import type { CapturedFrame } from './types'

type ReplaceTranslateOverlayProps = {
  frame: CapturedFrame
  cleanedImage: string
  groups: LensReplaceGroup[]
  slots: LensReplaceRenderSlot[]
  phase: 'ocr' | 'processing' | 'done' | 'error' | ''
  statusLabel: string
  // 局部降级详情（如修复回退、个别区域回退原文）：非错误，挂在状态胶囊 title 上。
  statusTitle?: string
  escHint: string
}

function textX(bounds: LensReplaceRenderSlot['bounds'], align: LensReplaceRenderSlot['align'], padding: number) {
  return align === 'center'
    ? bounds.x + bounds.width / 2
    : align === 'right'
      ? bounds.x + bounds.width - padding
      : bounds.x + padding
}

function anchoredTextX(slot: LensReplaceRenderSlot, padding: number) {
  return slot.align === 'left' && slot.flow === 'exact_line'
    ? slot.anchor.x
    : textX(slot.bounds, slot.align, padding)
}

function anchoredTextY(
  slot: LensReplaceRenderSlot,
  innerHeight: number,
  contentHeight: number,
  padding: number,
) {
  if (slot.flow === 'exact_line' || slot.verticalAlign === 'top') return slot.anchor.y
  return slot.bounds.y + padding + replaceTextVerticalOffset(slot.kind, innerHeight, contentHeight)
}

function drawNormalSlotText(
  ctx: CanvasRenderingContext2D,
  slot: LensReplaceRenderSlot,
  layout: ReplaceTextFlowSlotLayout,
  fontPx: number,
  lineHeight: number,
  padding: number,
) {
  const { bounds } = slot
  const innerHeight = Math.max(1, bounds.height - padding * 2)
  ctx.font = `${fontPx}px system-ui, "Segoe UI", sans-serif`
  ctx.fillStyle = slot.sourceColor
  ctx.textBaseline = 'top'
  ctx.textAlign = slot.align
  const x = anchoredTextX(slot, padding)
  let y = anchoredTextY(slot, innerHeight, layout.contentHeight, padding)
  for (const line of layout.lines) {
    ctx.fillText(line, x, y)
    y += lineHeight
  }
}

function drawSafelyScaledSlotText(
  ctx: CanvasRenderingContext2D,
  slot: LensReplaceRenderSlot,
  layout: ReplaceTextFlowSlotLayout,
  fontPx: number,
  lineHeight: number,
  safeScale: number,
  padding: number,
) {
  const offscreen = document.createElement('canvas')
  offscreen.width = Math.max(1, Math.ceil(slot.bounds.width / safeScale))
  offscreen.height = Math.max(1, Math.ceil(slot.bounds.height / safeScale))
  const offscreenCtx = offscreen.getContext('2d')
  if (!offscreenCtx) return
  const virtualPadding = padding / safeScale
  const innerHeight = Math.max(1, offscreen.height - virtualPadding * 2)
  offscreenCtx.font = `${fontPx}px system-ui, "Segoe UI", sans-serif`
  offscreenCtx.fillStyle = slot.sourceColor
  offscreenCtx.textBaseline = 'top'
  offscreenCtx.textAlign = slot.align
  const anchorX = (slot.anchor.x - slot.bounds.x) / safeScale
  const anchorY = (slot.anchor.y - slot.bounds.y) / safeScale
  const x = slot.align === 'left' && slot.flow === 'exact_line'
    ? anchorX
    : slot.align === 'center'
    ? offscreen.width / 2
    : slot.align === 'right'
      ? offscreen.width - virtualPadding
      : virtualPadding
  let y = slot.flow === 'exact_line' || slot.verticalAlign === 'top'
    ? anchorY
    : virtualPadding + replaceTextVerticalOffset(slot.kind, innerHeight, layout.contentHeight)
  for (const line of layout.lines) {
    offscreenCtx.fillText(line, x, y)
    y += lineHeight
  }
  ctx.drawImage(offscreen, slot.bounds.x, slot.bounds.y, slot.bounds.width, slot.bounds.height)
}

export function ReplaceTranslateOverlay({
  frame,
  cleanedImage,
  groups,
  slots,
  phase,
  statusLabel,
  statusTitle,
  escHint,
}: ReplaceTranslateOverlayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !cleanedImage || groups.length === 0 || slots.length === 0 || phase !== 'done') return
    const context = canvas.getContext('2d')
    if (!context) return
    let cancelled = false
    const image = new Image()
    image.onload = () => {
      if (cancelled) return
      canvas.width = Math.max(1, image.naturalWidth)
      canvas.height = Math.max(1, image.naturalHeight)
      context.clearRect(0, 0, canvas.width, canvas.height)
      context.drawImage(image, 0, 0)
      const slotsByGroup = new Map<string, LensReplaceRenderSlot[]>()
      for (const slot of slots) {
        const groupSlots = slotsByGroup.get(slot.groupId) ?? []
        groupSlots.push(slot)
        slotsByGroup.set(slot.groupId, groupSlots)
      }
      for (const group of groups) {
        const groupSlots = (slotsByGroup.get(group.id) ?? [])
          .sort((left, right) => left.anchor.y - right.anchor.y || left.anchor.x - right.anchor.x)
        if (groupSlots.length === 0) continue
        const text = group.translated.trim() || group.sourceText
        if (!text) continue
        const sourceFontPx = Math.max(...groupSlots.map(slot => slot.sourceFontPx))
        const padding = Math.max(2, Math.min(6, sourceFontPx * 0.2))
        const layout = layoutReplaceTextFlow(
          text,
          groupSlots.map(slot => ({
            width: Math.max(1, slot.bounds.width - padding * 2),
            height: Math.max(1, slot.bounds.height - padding * 2),
          })),
          sourceFontPx,
          (value, fontPx) => {
            context.font = `${fontPx}px system-ui, "Segoe UI", sans-serif`
            return context.measureText(value).width
          },
        )
        groupSlots.forEach((slot, index) => {
          const slotLayout = layout.slots[index]
          if (!slotLayout || slotLayout.lines.length === 0) return
          context.save()
          context.beginPath()
          context.rect(slot.bounds.x, slot.bounds.y, slot.bounds.width, slot.bounds.height)
          context.clip()
          if (layout.safeScale < 1) {
            drawSafelyScaledSlotText(context, slot, slotLayout, layout.fontPx, layout.lineHeight, layout.safeScale, padding)
          } else {
            drawNormalSlotText(context, slot, slotLayout, layout.fontPx, layout.lineHeight, padding)
          }
          context.restore()
        })
      }
    }
    image.src = cleanedImage
    return () => {
      cancelled = true
    }
  }, [cleanedImage, groups, phase, slots])

  const showOverlay = phase === 'done' && cleanedImage && groups.length > 0 && slots.length > 0
  return (
    <>
      <div className="absolute top-[calc(env(safe-area-inset-top,0px)+36px)] left-0 right-0 z-30 flex flex-col items-center pointer-events-none">
        <div
          className="px-3 py-1.5 rounded-full text-[12px] font-medium bg-black/70 text-white shadow-lg backdrop-blur-sm"
          title={statusTitle}
        >
          {statusLabel}
          {phase !== 'done' && phase !== 'error' && (
            <span className="ml-2 inline-block w-1.5 h-1.5 rounded-full bg-white animate-pulse align-middle" />
          )}
        </div>
        <p className="text-center text-[11px] text-white/80 mt-1 drop-shadow">{escHint}</p>
      </div>
      {showOverlay && (
        <div
          className="absolute z-20 rounded-md overflow-hidden pointer-events-none"
          style={{ left: frame.x, top: frame.y, width: frame.width, height: frame.height }}
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
