import { describe, expect, it } from 'vitest'
import { appendTranscriptDelta, commentaryContent, emptyVoiceTranscript, formatElapsed, takeVoiceTranscript, voiceOrbState } from './useVoiceCall'

describe('voice transcript', () => {
  it('assembles contiguous deltas and preserves chronological role turns in delegation requests', () => {
    let cursor = emptyVoiceTranscript()
    cursor = appendTranscriptDelta(cursor, 'user', '请')
    cursor = appendTranscriptDelta(cursor, 'user', '查询天气')
    cursor = appendTranscriptDelta(cursor, 'assistant', '我来')
    cursor = appendTranscriptDelta(cursor, 'assistant', '查一下。')
    cursor = appendTranscriptDelta(cursor, 'user', '还有明天')

    expect(takeVoiceTranscript(cursor).request).toBe('用户：请查询天气\n助手：我来查一下。\n用户：还有明天')
  })

  it('resets the cursor after taking a request, including empty deltas and empty requests', () => {
    const empty = emptyVoiceTranscript()
    expect(appendTranscriptDelta(empty, 'user', '')).toBe(empty)
    expect(takeVoiceTranscript(empty)).toEqual({ request: '', transcript: empty })

    const first = takeVoiceTranscript(appendTranscriptDelta(empty, 'user', '第一句'))
    expect(first.request).toBe('用户：第一句')
    const second = takeVoiceTranscript(appendTranscriptDelta(first.transcript, 'user', '第二句'))
    expect(second.request).toBe('用户：第二句')
    expect(takeVoiceTranscript(second.transcript).request).toBe('')
  })


  it('limits spoken commentary without splitting Unicode', () => {
    expect(commentaryContent('完成')).toBe('完成')
    for (const text of ['a'.repeat(1201), '语'.repeat(600), '😀'.repeat(400)]) {
      const result = commentaryContent(text)
      expect(new TextEncoder().encode(result).length).toBeLessThanOrEqual(500)
      expect(result.endsWith('…')).toBe(true)
      expect(result).not.toContain('�')
    }
  })


  it('formats the call clock like a meeting timer', () => {
    expect(formatElapsed(-3)).toBe('0:00')
    expect(formatElapsed(65.9)).toBe('1:05')
    expect(formatElapsed(3600 + 61)).toBe('1:01:01')
  })

  it('maps call state to the orb with connecting over working over muted', () => {
    expect(voiceOrbState('connecting', true, 2)).toBe('connecting')
    expect(voiceOrbState('closing', false, 0)).toBe('connecting')
    expect(voiceOrbState('active', true, 1)).toBe('working')
    expect(voiceOrbState('active', true, 0)).toBe('breathing')
    expect(voiceOrbState('active', false, 0)).toBe('listening')
  })
})
