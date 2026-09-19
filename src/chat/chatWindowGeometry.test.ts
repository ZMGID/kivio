import { describe, expect, it } from 'vitest'
import { CHAT_MIN_SIZE_COLLAPSED } from './persistence'
import { chatWindowMinSize } from './chatWindowGeometry'

describe('chatWindowMinSize', () => {
  it('uses the collapsed constant when the sidebar is hidden', () => {
    expect(chatWindowMinSize(true, 320)).toEqual(CHAT_MIN_SIZE_COLLAPSED)
  })

  it('adds sidebar width to the collapsed min when the sidebar is open', () => {
    expect(chatWindowMinSize(false, 280)).toEqual({
      width: CHAT_MIN_SIZE_COLLAPSED.width + 280,
      height: CHAT_MIN_SIZE_COLLAPSED.height,
    })
  })
})
