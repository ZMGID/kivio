import { useEffect, useState } from 'react'
import { prefersReducedMotion } from './utils'

interface TypewriterTextProps {
  text: string
  active?: boolean
  resetKey?: string | null
  className?: string
  charDelayMs?: number
  startDelayMs?: number
}

export function TypewriterText({
  text,
  active = true,
  resetKey = null,
  className,
  charDelayMs = 42,
  startDelayMs = 120,
}: TypewriterTextProps) {
  const [displayed, setDisplayed] = useState('')
  const [done, setDone] = useState(false)

  useEffect(() => {
    if (!active) {
      setDisplayed('')
      setDone(true)
      return
    }

    if (prefersReducedMotion() || !text) {
      setDisplayed(text)
      setDone(true)
      return
    }

    setDisplayed('')
    setDone(false)
    let index = 0
    let charTimer: ReturnType<typeof setTimeout> | undefined

    const typeNext = () => {
      index += 1
      setDisplayed(text.slice(0, index))
      if (index < text.length) {
        const ch = text[index - 1]
        const pause = ch === ' ' || ch === '—' || ch === '?' || ch === '.' ? charDelayMs + 80 : charDelayMs
        charTimer = setTimeout(typeNext, pause)
      } else {
        setDone(true)
      }
    }

    const startTimer = setTimeout(typeNext, startDelayMs)

    return () => {
      clearTimeout(startTimer)
      clearTimeout(charTimer)
    }
  }, [text, active, resetKey, charDelayMs, startDelayMs])

  return (
    <span className={className}>
      {displayed}
      {active && !done && (
        <span className="chat-typewriter-cursor" aria-hidden="true">
          |
        </span>
      )}
    </span>
  )
}
