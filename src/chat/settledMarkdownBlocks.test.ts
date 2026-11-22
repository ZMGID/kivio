import { createElement } from 'react'
import { expect, it } from 'vitest'
import { readSettledBlock, rememberSettledBlock } from './settledMarkdownBlocks'

it('evicts least recently viewed blocks when dense render trees fill the cache', () => {
  const table = createElement('table', null, Array.from({ length: 980 },
    (_, row) => createElement('tr', { key: row }, Array.from({ length: 4 }, (_, col) => createElement('td', { key: col }, 'x')))))
  rememberSettledBlock('old', 'first paragraph')
  rememberSettledBlock('recent', 'last paragraph')
  for (let i = 0; i < 4; i++) rememberSettledBlock('table ' + i, table)
  expect(readSettledBlock('recent')).toBe('last paragraph')
  rememberSettledBlock('another table', table)
  expect(readSettledBlock('old')).toBeUndefined()
  expect(readSettledBlock('recent')).toBe('last paragraph')
})
