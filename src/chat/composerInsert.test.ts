import { describe, expect, it } from 'vitest'
import {
  insertIntoComposer,
  insertTextIntoComposer,
  onComposerInsert,
  onComposerTextInsert,
} from './composerInsert'
import { getComposerDraft, setComposerDraft } from './composerDraft'
import type { PendingAttachment } from './types'

describe('composer insertion across cached pages', () => {
  it('restores images into an existing draft without losing text, quotes or attachments', () => {
    const key = 'insert-images'
    const image: PendingAttachment = { id: 'image', type: 'image', name: 'one.png', path: 'C:/chat/one.png' }
    const existing: PendingAttachment = { id: 'file', type: 'file', name: 'draft.txt', path: 'C:/draft.txt' }
    setComposerDraft(key, { input: 'draft', quotes: ['quote'], attachments: [existing] })
    const leave = onComposerTextInsert(() => {}, key)
    try {
      insertTextIntoComposer('', [image])
      insertTextIntoComposer('', [image])
      expect(getComposerDraft(key)).toEqual({ input: 'draft', quotes: ['quote'], attachments: [existing, image] })
    } finally {
      leave()
      setComposerDraft(key, { input: '', quotes: [], attachments: [] })
    }
  })

  it('returns quote insertion to the cached chat after the reading composer leaves', () => {
    const chat: string[] = [],
      reading: string[] = []
    const leaveChat = onComposerInsert((text) => chat.push(text))
    const leaveReading = onComposerInsert((text) => reading.push(text))
    try {
      insertIntoComposer('selected source')
      leaveReading()
      insertIntoComposer('chat quote')
      expect(reading).toEqual(['selected source'])
      expect(chat).toEqual(['chat quote'])
    } finally {
      leaveReading()
      leaveChat()
    }
  })

  it('keeps text and drafts scoped through restoration and out-of-order cleanup', () => {
    const chat: string[] = [],
      reading: string[] = []
    const leaveChat = onComposerTextInsert(
      (text) => chat.push(text),
      'insert-test-chat',
    )
    const leaveReading = onComposerTextInsert(
      (text) => reading.push(text),
      'insert-test-reading',
    )
    let leaveReplacement = () => {}
    try {
      insertTextIntoComposer('source')
      leaveReading()
      insertTextIntoComposer('question')
      leaveReplacement = onComposerTextInsert(
        (text) => reading.push(text),
        'insert-test-reading',
      )
      leaveChat()
      insertTextIntoComposer('more')
      expect(chat).toEqual(['question'])
      expect(reading).toEqual(['source', 'more'])
      expect(getComposerDraft('insert-test-chat')?.input).toBe('question')
      expect(getComposerDraft('insert-test-reading')?.input).toBe('source more')
    } finally {
      leaveReplacement()
      leaveReading()
      leaveChat()
      for (const key of ['insert-test-chat', 'insert-test-reading'])
        setComposerDraft(key, { input: '', quotes: [], attachments: [] })
    }
  })
})
