import { useEffect, useMemo, useState } from 'react'
import { api } from './api/tauri'

// 点坐标类型
type Point = { x: number; y: number }
// 矩形区域类型
type Rect = { x: number; y: number; width: number; height: number }

// 最小框选尺寸阈值（小于此值视为无效选择）
const minSize = 6

/**
 * 计算两个点构成的矩形区域
 * @param start 起始点
 * @param current 当前点
 * @returns 矩形区域对象
 */
const clampRect = (start: Point, current: Point): Rect => {
  const x = Math.min(start.x, current.x)
  const y = Math.min(start.y, current.y)
  const width = Math.abs(current.x - start.x)
  const height = Math.abs(current.y - start.y)
  return { x, y, width, height }
}

/**
 * 截图区域选择覆盖层组件
 * 用于 Windows 平台的区域截图功能
 * 用户在全屏透明覆盖层上拖拽选择截图区域
 */
export default function CaptureOverlay() {
  // 是否正在拖拽中
  const [dragging, setDragging] = useState(false)
  // 拖拽起始点
  const [startPoint, setStartPoint] = useState<Point | null>(null)
  // 当前鼠标位置
  const [currentPoint, setCurrentPoint] = useState<Point | null>(null)
  // 是否正在提交截图区域
  const [submitting, setSubmitting] = useState(false)

  // 重置所有选择状态
  const resetSelection = () => {
    setDragging(false)
    setStartPoint(null)
    setCurrentPoint(null)
    setSubmitting(false)
  }

  // 根据起始点和当前点计算矩形区域
  const rect = useMemo(() => {
    if (!startPoint || !currentPoint) return null
    return clampRect(startPoint, currentPoint)
  }, [startPoint, currentPoint])

  useEffect(() => {
    // Esc 键取消截图
    const handleEsc = async (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      await api.captureCancel()
    }
    // 监听重置事件（后端触发）
    const handleReset = () => resetSelection()

    window.addEventListener('keydown', handleEsc)
    window.addEventListener('capture:reset', handleReset)
    return () => {
      window.removeEventListener('keydown', handleEsc)
      window.removeEventListener('capture:reset', handleReset)
    }
  }, [])

  useEffect(() => {
    resetSelection()
  }, [])

  // 将鼠标事件转换为页面坐标点
  const toPagePoint = (event: React.MouseEvent): Point => ({
    x: Math.round(event.clientX),
    y: Math.round(event.clientY),
  })

  // 鼠标按下：开始拖拽
  const handleMouseDown = (event: React.MouseEvent) => {
    if (submitting) return
    const p = toPagePoint(event)
    setDragging(true)
    setStartPoint(p)
    setCurrentPoint(p)
  }

  // 鼠标移动：更新当前点
  const handleMouseMove = (event: React.MouseEvent) => {
    if (!dragging || submitting) return
    setCurrentPoint(toPagePoint(event))
  }

  // 鼠标释放：确认选择区域并提交给后端
  const handleMouseUp = async (event: React.MouseEvent) => {
    if (!dragging || submitting) return
    setDragging(false)
    const end = toPagePoint(event)
    setCurrentPoint(end)

    // 计算最终矩形区域
    const currentRect = startPoint ? clampRect(startPoint, end) : null
    // 如果区域太小则视为无效，取消选择
    if (!currentRect || currentRect.width < minSize || currentRect.height < minSize) {
      setStartPoint(null)
      setCurrentPoint(null)
      return
    }

    try {
      setSubmitting(true)
      // 将逻辑坐标转换为屏幕绝对坐标提交给 Rust 后端
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
      {/* 半透明黑色遮罩层 */}
      <div className="absolute inset-0 bg-black/35" />

      {/* 高亮选中区域 */}
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

      {/* 顶部提示文字 */}
      <div className="absolute top-5 left-1/2 -translate-x-1/2 rounded-md bg-black/70 px-3 py-1 text-[12px] text-white">
        拖拽框选截图区域，按 Esc 取消
      </div>
    </div>
  )
}
