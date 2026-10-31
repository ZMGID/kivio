import { useLayoutEffect, useRef } from 'react'
import type { VoiceTranscriptRow } from './useVoiceCall'

export function VoiceCaption({ row, placeholder, label }: {
  row?: VoiceTranscriptRow
  placeholder: string
  label: string
}) {
  const lineRef = useRef<HTMLDivElement>(null)
  const textRef = useRef<HTMLSpanElement>(null)
  const turnId = row?.id
  const text = row?.text || placeholder

  useLayoutEffect(() => {
    const line = lineRef.current
    if (line) line.scrollLeft = Math.max(0, line.scrollWidth - line.clientWidth)
  }, [text, turnId])

  useLayoutEffect(() => {
    const line = lineRef.current
    const content = textRef.current
    if (!line || !content || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(() => {
      line.scrollLeft = Math.max(0, line.scrollWidth - line.clientWidth)
    })
    observer.observe(line)
    observer.observe(content)
    return () => observer.disconnect()
  }, [turnId])

  return <div className="kv-voice-caption" role="status" aria-label={label}>
    <div className="kv-voice-caption-line" ref={lineRef}>
      <span className="kv-voice-caption-text" ref={textRef} key={row ? `turn:${row.id}` : 'placeholder'}>{text}</span>
    </div>
  </div>
}
