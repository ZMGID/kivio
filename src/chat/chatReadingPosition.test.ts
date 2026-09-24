import { afterEach, expect, it } from 'vitest'
import { clearChatReadingPositions, recallChatReadingPosition, rememberChatReadingPosition } from './chatReadingPosition'

afterEach(clearChatReadingPositions)

it('keeps only recent conversation reading positions', () => {
  const position = { following: false, rowKey: 'message-1', rowOffset: 24, scrollTop: 400, layoutKey: 'wide' }
  rememberChatReadingPosition('first', position)
  for (let index = 0; index < 24; index += 1) {
    rememberChatReadingPosition(String(index), { ...position, rowKey: String(index) })
  }
  expect(recallChatReadingPosition('first')).toBeNull()
  expect(recallChatReadingPosition('23')?.rowKey).toBe('23')
})
