// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import './browserDefaults'

describe('native browser defaults', () => {
  it.each([
    { key: 'f', ctrlKey: true },
    { key: 'F', ctrlKey: true, shiftKey: true },
    { key: 'f', metaKey: true },
    { key: 'g', ctrlKey: true },
    { key: 'G', ctrlKey: true, shiftKey: true },
    { key: 'F3' },
    { key: 'F3', shiftKey: true },
  ])('prevents native find for %j without swallowing app search', (keys) => {
    const input = document.createElement('input')
    document.body.append(input)
    const event = new KeyboardEvent('keydown', { ...keys, bubbles: true, cancelable: true })
    let reachedApp = false
    input.addEventListener('keydown', (event) => {
      reachedApp = true
      event.stopPropagation()
    })
    input.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
    expect(reachedApp).toBe(true)
    input.remove()
  })

  it.each([
    { key: 'f' },
    { key: 'c', ctrlKey: true },
    { key: 'v', metaKey: true },
    { key: 'Enter', ctrlKey: true },
    { key: 'f', ctrlKey: true, altKey: true },
    { key: 'f', ctrlKey: true, isComposing: true },
  ])('preserves typing and other app shortcuts: %j', (keys) => {
    const event = new KeyboardEvent('keydown', { ...keys, bubbles: true, cancelable: true })
    document.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(false)
  })

  it('keeps the native context menu disabled', () => {
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    document.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
  })
})
