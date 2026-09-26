// @vitest-environment jsdom

import { afterEach, describe, expect, it } from 'vitest'
import {
  clearRememberedChatHeadingOutlineState,
  getRememberedChatHeadingOutlineExpanded,
  rememberChatHeadingOutlineExpanded,
} from './chatHeadingOutlinePersistence'

describe('chat heading outline persistence', () => {
  afterEach(() => {
    clearRememberedChatHeadingOutlineState()
  })

  it('stores level choices independently by conversation', () => {
    rememberChatHeadingOutlineExpanded('first', true)
    rememberChatHeadingOutlineExpanded('second', false)

    expect(getRememberedChatHeadingOutlineExpanded('first')).toBe(true)
    expect(getRememberedChatHeadingOutlineExpanded('second')).toBe(false)
    expect(getRememberedChatHeadingOutlineExpanded('missing')).toBe(false)
  })

  it('ignores malformed storage and caps old conversation entries', () => {
    window.localStorage.setItem('kivio-chat-heading-outline-levels', '{bad json')
    expect(getRememberedChatHeadingOutlineExpanded('first')).toBe(false)

    for (let index = 0; index < 51; index += 1) {
      rememberChatHeadingOutlineExpanded(String(index), true)
    }
    expect(getRememberedChatHeadingOutlineExpanded('0')).toBe(false)
    expect(getRememberedChatHeadingOutlineExpanded('50')).toBe(true)
  })
})
