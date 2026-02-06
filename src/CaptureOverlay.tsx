import { useEffect, useMemo, useState } from 'react'
import { api } from './api/tauri'

type Point = { x: number; y: number }
type Rect = { x: number; y: number; width: number; height: number }

const minSize = 6

const clampRect = (start: Point, current: Point): Rect => {
  const x = Math.min(start.x, current.x)
  const y = Math.min(start.y, current.y)
  const width = Math.abs(current.x - start.x)
  const height = Math.abs(current.y - start.y)
  return { x, y, width, height }
}

export default function CaptureOverlay() {
  const [dragging, setDragging] = useState(false)
  const [startPoint, setStartPoint] = useState<Point | null>(null)
  const [currentPoint, setCurrentPoint] = useState<Point | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const resetSelection = () => {
    setDragging(false)
    setStartPoint(null)
    setCurrentPoint(null)
    setSubmitting(false)
  }

  const rect = useMemo(() => {
    if (!startPoint || !currentPoint) return null
    return clampRect(startPoint, currentPoint)
  }, [startPoint, currentPoint])

  useEffect(() => {
    const handleEsc = async (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      await api.captureCancel()
    }
    window.addEventListener('keydown', handleEsc)
    const handleReset = () => resetSelection()
    window.addEventListener('capture:reset', handleReset)
    return () => {
      window.removeEventListener('keydown', handleEsc)
      window.removeEventListener('capture:reset', handleReset)
    }
  }, [])

  useEffect(() => {
    resetSelection()
  }, [])

  const toPagePoint = (event: React.MouseEvent): Point => ({
    x: Math.round(event.clientX),
    y: Math.round(event.clientY),
  })

  const handleMouseDown = (event: React.MouseEvent) => {
    if (submitting) return
    const p = toPagePoint(event)
    setDragging(true)
    setStartPoint(p)
    setCurrentPoint(p)
  }

  const handleMouseMove = (event: React.MouseEvent) => {
    if (!dragging || submitting) return
    setCurrentPoint(toPagePoint(event))
  }

  const handleMouseUp = async (event: React.MouseEvent) => {
    if (!dragging || submitting) return
    setDragging(false)
    const end = toPagePoint(event)
    setCurrentPoint(end)

    const currentRect = startPoint ? clampRect(startPoint, end) : null
    if (!currentRect || currentRect.width < minSize || currentRect.height < minSize) {
      setStartPoint(null)
      setCurrentPoint(null)
      return
    }

    try {
      setSubmitting(true)
      await api.captureCommit({
        absoluteX: Math.round(window.screenX + currentRect.x),
        absoluteY: Math.round(window.screenY + currentRect.y),
        x: currentRect.x,
        y: currentRect.y,
        width: currentRect.width,
        height: currentRect.height,
        scaleFactor: window.devicePixelRatio || 1,
      })
    } catch (err) {
      console.error('Failed to commit capture', err)
      await api.captureCancel()
    } finally {
      resetSelection()
    }
  }

  return (
    <div
      className="fixed inset-0 cursor-crosshair select-none"
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      data-tauri-drag-region="false"
    >
      <div className="absolute inset-0 bg-black/35" />

      {rect && (
        <div
          className="absolute border border-white shadow-[0_0_0_9999px_rgba(0,0,0,0.35)]"
          style={{
            left: rect.x,
            top: rect.y,
            width: rect.width,
            height: rect.height,
          }}
        />
      )}

      <div className="absolute top-5 left-1/2 -translate-x-1/2 rounded-md bg-black/70 px-3 py-1 text-[12px] text-white">
        拖拽框选截图区域，按 Esc 取消
      </div>
    </div>
  )
}
