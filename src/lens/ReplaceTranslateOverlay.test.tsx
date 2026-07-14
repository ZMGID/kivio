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
})
