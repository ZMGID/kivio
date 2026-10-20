import { expect, it } from 'vitest'
import { readingCitation } from './readingContext'

it('resolves only existing PDF pages', () => {
  expect(readingCitation('#reading-page-2', 2)).toBe(2)
  expect(readingCitation('#reading-page-3', 2)).toBeNull()
  expect(readingCitation('https://example.com/#other', 2)).toBeNull()
})
