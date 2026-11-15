import { afterEach, expect, it } from 'vitest'
import {
  clearChatReadingPositions, readingGroupRowKey, readingPositionNeedsFullHistory,
  recallChatReadingPosition, rememberChatReadingPosition,
} from './chatReadingPosition'

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

it('asks for full history only when the row the reader left at is outside the loaded window', () => {
  const position = { following: false, rowKey: 'answer-9', rowOffset: 0, scrollTop: 0, layoutKey: 'wide' }
  const loaded = [{ id: 'question-9' }, { id: 'answer-9' }, { id: 'arm-a', group_id: 'g1' }]
  expect(readingPositionNeedsFullHistory(position, loaded)).toBe(false)
  expect(readingPositionNeedsFullHistory({ ...position, rowKey: readingGroupRowKey('g1') }, loaded)).toBe(false)
  expect(readingPositionNeedsFullHistory({ ...position, rowKey: 'answer-2' }, loaded)).toBe(true)
  expect(readingPositionNeedsFullHistory({ ...position, rowKey: null }, loaded)).toBe(true)
  expect(readingPositionNeedsFullHistory({ ...position, following: true, rowKey: 'answer-2' }, loaded)).toBe(false)
  expect(readingPositionNeedsFullHistory({ ...position, rowKey: 'live-answer' }, loaded, 'live-answer')).toBe(false)
  expect(readingPositionNeedsFullHistory(null, loaded)).toBe(false)
})
