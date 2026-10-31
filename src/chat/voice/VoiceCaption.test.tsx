import { act, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { VoiceCaption } from './VoiceCaption'
import type { VoiceTranscriptRow } from './useVoiceCall'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('VoiceCaption', () => {
  it('updates a single accessible caption in place as a turn streams and finalizes', () => {
    const row: VoiceTranscriptRow = { id: 'answer', role: 'unknown', text: '你好 👩‍💻', finalized: false }
    const { rerender } = render(<VoiceCaption row={row} placeholder="Listening" label="Live captions" />)
    const status = screen.getByRole('status', { name: 'Live captions' })
    const text = status.querySelector('.kv-voice-caption-text')
    const textNode = text?.firstChild
    expect(status.textContent).toBe('你好 👩‍💻')
    expect(screen.getAllByRole('status')).toHaveLength(1)

    rerender(<VoiceCaption row={{ ...row, role: 'assistant', text: '你好 👩‍💻，继续说。\n第二句', finalized: true }} placeholder="Listening" label="Live captions" />)
    expect(screen.getByRole('status', { name: 'Live captions' })).toBe(status)
    expect(status.querySelector('.kv-voice-caption-text')).toBe(text)
    expect(text?.firstChild).toBe(textNode)
    expect(status.textContent).toBe('你好 👩‍💻，继续说。\n第二句')
    expect(screen.queryByText('Listening')).not.toBeInTheDocument()
  })

  it('shows the placeholder for an absent or empty turn, including an empty placeholder', () => {
    const row: VoiceTranscriptRow = { id: 'user', role: 'user', text: '', finalized: false }
    const { rerender } = render(<VoiceCaption placeholder="" label="Live captions" />)
    const status = screen.getByRole('status', { name: 'Live captions' })
    expect(status.textContent).toBe('')
    rerender(<VoiceCaption row={row} placeholder="Listening" label="Live captions" />)
    expect(status.textContent).toBe('Listening')
    rerender(<VoiceCaption row={{ ...row, text: 'Hello' }} placeholder="Listening" label="Live captions" />)
    expect(status.textContent).toBe('Hello')
    rerender(<VoiceCaption placeholder="" label="Live captions" />)
    expect(status.textContent).toBe('')
  })

  it('follows streamed and resized text at the tail and resets on a new turn', () => {
    const callbacks: ResizeObserverCallback[] = []
    const observe = vi.fn()
    const disconnect = vi.fn()
    vi.stubGlobal('ResizeObserver', class {
      observe = observe
      disconnect = disconnect
      constructor(callback: ResizeObserverCallback) { callbacks.push(callback) }
    })
    let width = 720
    let viewportWidth = 200
    vi.spyOn(Element.prototype, 'scrollWidth', 'get').mockImplementation(() => width)
    vi.spyOn(Element.prototype, 'clientWidth', 'get').mockImplementation(() => viewportWidth)
    const row: VoiceTranscriptRow = { id: 'user', role: 'user', text: '很长的语音内容 👩‍💻 '.repeat(80), finalized: false }
    const { rerender, unmount } = render(<VoiceCaption row={row} placeholder="" label="Live captions" />)
    const status = screen.getByRole('status', { name: 'Live captions' })
    const line = status.querySelector('.kv-voice-caption-line') as HTMLElement
    const text = status.querySelector('.kv-voice-caption-text')
    expect(line.scrollLeft).toBe(520)
    expect(status.textContent).toBe(row.text)

    width = 900
    rerender(<VoiceCaption row={{ ...row, text: `${row.text} 还有最后一句` }} placeholder="" label="Live captions" />)
    expect(line.scrollLeft).toBe(700)
    expect(status.textContent).toBe(`${row.text} 还有最后一句`)
    expect(status.querySelector('.kv-voice-caption-text')).toBe(text)

    viewportWidth = 300
    act(() => callbacks[0]([], {} as ResizeObserver))
    expect(line.scrollLeft).toBe(600)
    width = 80
    rerender(<VoiceCaption row={{ id: 'answer', role: 'assistant', text: '好', finalized: false }} placeholder="" label="Live captions" />)
    expect(line.scrollLeft).toBe(0)
    expect(status.textContent).toBe('好')
    expect(status.querySelector('.kv-voice-caption-text')).not.toBe(text)
    expect(disconnect).toHaveBeenCalledOnce()

    width = 260
    viewportWidth = 100
    act(() => callbacks[1]([], {} as ResizeObserver))
    expect(line.scrollLeft).toBe(160)
    unmount()
    expect(disconnect).toHaveBeenCalledTimes(2)
  })

  it('still follows updates without ResizeObserver', () => {
    vi.stubGlobal('ResizeObserver', undefined)
    let width = 400
    vi.spyOn(Element.prototype, 'scrollWidth', 'get').mockImplementation(() => width)
    vi.spyOn(Element.prototype, 'clientWidth', 'get').mockReturnValue(100)
    const row: VoiceTranscriptRow = { id: 'one', role: 'user', text: 'Hello', finalized: false }
    const { rerender } = render(<VoiceCaption row={row} placeholder="" label="Live captions" />)
    const line = screen.getByRole('status').querySelector('.kv-voice-caption-line') as HTMLElement
    expect(line.scrollLeft).toBe(300)
    width = 500
    rerender(<VoiceCaption row={{ ...row, text: 'Hello again' }} placeholder="" label="Live captions" />)
    expect(line.scrollLeft).toBe(400)
  })
})
