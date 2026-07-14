import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ReplaceTranslateOverlay } from './ReplaceTranslateOverlay'

describe('ReplaceTranslateOverlay status', () => {
  it('renders the localized status label instead of an internal error code', () => {
    render(
      <ReplaceTranslateOverlay
        frame={{ x: 0, y: 0, width: 320, height: 180, label: 'fixture' }}
        cleanedImage=""
        groups={[]}
        slots={[]}
        phase="done"
        statusLabel="替换翻译离线包未下载。请在设置中下载。"
        escHint="按 Esc 关闭"
      />,
    )

    const status = screen.getByText('替换翻译离线包未下载。请在设置中下载。')
    expect(status).toBeInTheDocument()
    expect(status.parentElement).toHaveClass('top-[calc(env(safe-area-inset-top,0px)+36px)]')
    expect(screen.queryByText('replace_translation_pack_missing')).not.toBeInTheDocument()
  })

  it('clips the replacement canvas to the captured-frame rounded corners', () => {
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
    const { container } = render(
      <ReplaceTranslateOverlay
        frame={{ x: 0, y: 0, width: 320, height: 180, label: 'fixture' }}
        cleanedImage="data:image/png;base64,fixture"
        groups={[{
          id: 'r0000',
          leafIds: ['s0000'],
          sourceText: 'source',
          translated: '译文',
        }]}
        slots={[{
          id: 'r0000-s00',
          groupId: 'r0000',
          leafIds: ['s0000'],
          bounds: { x: 10, y: 10, width: 100, height: 30 },
          anchor: { x: 10, y: 10, baselineY: 25 },
          flow: 'exact_line',
          kind: 'line',
          align: 'left',
          verticalAlign: 'center',
          sourceFontPx: 16,
          sourceColor: '#111827',
        }]}
        phase="done"
        statusLabel="完成"
        escHint="按 Esc 关闭"
      />,
    )

    expect(container.querySelector('canvas')?.parentElement).toHaveClass('rounded-md', 'overflow-hidden')
    getContext.mockRestore()
  })

  it('keeps exact-line slots top anchored instead of vertically centering translated text', () => {
    const fillText = vi.fn()
    const context = {
      clearRect: vi.fn(),
      drawImage: vi.fn(),
      measureText: (text: string) => ({ width: text.length * 8 }),
      save: vi.fn(),
      beginPath: vi.fn(),
      rect: vi.fn(),
      clip: vi.fn(),
      restore: vi.fn(),
      fillText,
      font: '',
      fillStyle: '',
      textBaseline: '',
      textAlign: '',
    } as unknown as CanvasRenderingContext2D
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context)
    const originalImage = globalThis.Image
    class FixtureImage {
      naturalWidth = 320
      naturalHeight = 180
      onload: (() => void) | null = null
      set src(_value: string) {
        this.onload?.()
      }
    }
    globalThis.Image = FixtureImage as unknown as typeof Image

    render(
      <ReplaceTranslateOverlay
        frame={{ x: 0, y: 0, width: 320, height: 180, label: 'fixture' }}
        cleanedImage="data:image/png;base64,fixture"
        groups={[{ id: 'r0000', leafIds: ['s0000'], sourceText: 'source', translated: '短译文' }]}
        slots={[{
          id: 'r0000-s00',
          groupId: 'r0000',
          leafIds: ['s0000'],
          bounds: { x: 8, y: 16, width: 180, height: 60 },
          anchor: { x: 12, y: 20, baselineY: 36 },
          flow: 'exact_line',
          kind: 'line',
          align: 'left',
          verticalAlign: 'top',
          sourceFontPx: 16,
          sourceColor: '#111827',
        }]}
        phase="done"
        statusLabel="完成"
        escHint="按 Esc 关闭"
      />,
    )

    expect(fillText).toHaveBeenCalledWith('短译文', 12, 20)
    globalThis.Image = originalImage
    getContext.mockRestore()
  })

  it('sizes the canvas backing store to the cleaned image natural size, not the CSS frame size', () => {
    const context = {
      clearRect: vi.fn(),
      drawImage: vi.fn(),
      measureText: (text: string) => ({ width: text.length * 8 }),
      save: vi.fn(),
      beginPath: vi.fn(),
      rect: vi.fn(),
      clip: vi.fn(),
      restore: vi.fn(),
      fillText: vi.fn(),
      font: '',
      fillStyle: '',
      textBaseline: '',
      textAlign: '',
    } as unknown as CanvasRenderingContext2D
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context)
    const originalImage = globalThis.Image
    // Cleaned image is a Retina-scale 640×360 while the on-screen frame is 320×180 CSS px.
    class FixtureImage {
      naturalWidth = 640
      naturalHeight = 360
      onload: (() => void) | null = null
      set src(_value: string) {
        this.onload?.()
      }
    }
    globalThis.Image = FixtureImage as unknown as typeof Image

    const { container } = render(
      <ReplaceTranslateOverlay
        frame={{ x: 0, y: 0, width: 320, height: 180, label: 'fixture' }}
        cleanedImage="data:image/png;base64,fixture"
        groups={[{ id: 'r0000', leafIds: ['s0000'], sourceText: 'source', translated: '短译文' }]}
        slots={[{
          id: 'r0000-s00',
          groupId: 'r0000',
          leafIds: ['s0000'],
          bounds: { x: 8, y: 16, width: 180, height: 60 },
          anchor: { x: 12, y: 20, baselineY: 36 },
          flow: 'exact_line',
          kind: 'line',
          align: 'left',
          verticalAlign: 'top',
          sourceFontPx: 16,
          sourceColor: '#111827',
        }]}
        phase="done"
        statusLabel="完成"
        escHint="按 Esc 关闭"
      />,
    )

    const canvas = container.querySelector('canvas')
    // Backing store follows the cleaned image's natural pixels so geometry stays aligned with OCR coords.
    expect(canvas?.width).toBe(640)
    expect(canvas?.height).toBe(360)
    // CSS box still displays at the captured-frame logical size.
    expect(canvas?.style.width).toBe('320px')
    expect(canvas?.style.height).toBe('180px')
    globalThis.Image = originalImage
    getContext.mockRestore()
  })

  it('renders a scene_patch slot as complete system-font text (deterministic photo fallback)', () => {
    // scene-rendering baseline: until a gated photo redraw model + rotation
    // threading land, a PhotoText region degrades to the plain bounds-anchored
    // system-font path — content must stay complete, never dropped.
    const fillText = vi.fn()
    const context = {
      clearRect: vi.fn(),
      drawImage: vi.fn(),
      measureText: (text: string) => ({ width: text.length * 8 }),
      save: vi.fn(),
      beginPath: vi.fn(),
      rect: vi.fn(),
      clip: vi.fn(),
      restore: vi.fn(),
      fillText,
      font: '',
      fillStyle: '',
      textBaseline: '',
      textAlign: '',
    } as unknown as CanvasRenderingContext2D
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context)
    const originalImage = globalThis.Image
    class FixtureImage {
      naturalWidth = 320
      naturalHeight = 180
      onload: (() => void) | null = null
      set src(_value: string) {
        this.onload?.()
      }
    }
    globalThis.Image = FixtureImage as unknown as typeof Image

    render(
      <ReplaceTranslateOverlay
        frame={{ x: 0, y: 0, width: 320, height: 180, label: 'fixture' }}
        cleanedImage="data:image/png;base64,fixture"
        groups={[{ id: 'r0000', leafIds: ['s0000'], sourceText: 'SALE', translated: '促销' }]}
        slots={[{
          id: 'r0000-s00',
          groupId: 'r0000',
          leafIds: ['s0000'],
          bounds: { x: 40, y: 50, width: 180, height: 60 },
          anchor: { x: 44, y: 54, baselineY: 78 },
          flow: 'scene_patch',
          kind: 'line',
          align: 'left',
          verticalAlign: 'top',
          sourceFontPx: 28,
          sourceColor: '#ffffff',
        }]}
        phase="done"
        statusLabel="完成"
        escHint="按 Esc 关闭"
      />,
    )

    // Full translated text is drawn (not dropped, not truncated).
    expect(fillText).toHaveBeenCalledWith('促销', expect.any(Number), expect.any(Number))
    globalThis.Image = originalImage
    getContext.mockRestore()
  })
})
