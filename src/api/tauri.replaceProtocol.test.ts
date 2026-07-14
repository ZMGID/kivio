import { describe, expect, it } from 'vitest'
import { parseLensReplaceStreamPayload } from './tauri'

const donePayload = {
  version: 2,
  imageId: 'image-1',
  phase: 'done',
  groups: [{
    id: 'r0000',
    leafIds: ['s0000', 's0001'],
    sourceText: 'first\nsecond',
    translated: '第一行和第二行',
  }],
  slots: [
    {
      id: 'r0000-s00',
      groupId: 'r0000',
      leafIds: ['s0000'],
      bounds: { x: 10, y: 20, width: 120, height: 24 },
      anchor: { x: 12, y: 20, baselineY: 39 },
      flow: 'paragraph_flow',
      kind: 'paragraph',
      align: 'left',
      verticalAlign: 'top',
      sourceFontPx: 14,
      sourceColor: '#111827',
    },
    {
      id: 'r0000-s01',
      groupId: 'r0000',
      leafIds: ['s0001'],
      bounds: { x: 10, y: 48, width: 120, height: 24 },
      anchor: { x: 12, y: 48, baselineY: 67 },
      flow: 'paragraph_flow',
      kind: 'paragraph',
      align: 'left',
      verticalAlign: 'top',
      sourceFontPx: 14,
      sourceColor: '#111827',
    },
  ],
  cleanedImage: 'data:image/png;base64,fixture',
  warning: null,
  error: null,
}

describe('lens replace protocol v2', () => {
  it('decodes groups and independent render slots at the event boundary', () => {
    const parsed = parseLensReplaceStreamPayload(donePayload)
    expect(parsed.groups).toHaveLength(1)
    expect(parsed.slots.map(slot => slot.anchor.y)).toEqual([20, 48])
  })

  it('rejects the removed regions-only payload', () => {
    expect(() => parseLensReplaceStreamPayload({
      imageId: 'image-1',
      phase: 'done',
      regions: [],
    })).toThrow('version 2')
  })

  it('rejects slots that reference an unknown translation group', () => {
    expect(() => parseLensReplaceStreamPayload({
      ...donePayload,
      slots: [{ ...donePayload.slots[0], groupId: 'missing' }],
    })).toThrow('groupId is unknown')
  })
})
