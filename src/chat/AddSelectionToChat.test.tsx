// @vitest-environment jsdom
import { useEffect, useState } from 'react'
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { AddSelectionToChat } from './AddSelectionToChat'
import { MessageGroup } from './MessageGroup'
import { onComposerInsert } from './composerInsert'
import { useMultiAnswerViewMode, type MultiAnswerViewMode } from './multiAnswerViewMode'

function setMode(mode: MultiAnswerViewMode) {
  const { result, unmount } = renderHook(() => useMultiAnswerViewMode())
  act(() => result.current[1](mode))
  unmount()
}

function selectText(element: HTMLElement, start = 0, end = element.textContent!.length) {
  const range = document.createRange()
  range.setStart(element.firstChild!, start)
  range.setEnd(element.firstChild!, end)
  range.getBoundingClientRect = () => new DOMRect(100, 100, 180, 20)
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
  fireEvent(document, new Event('selectionchange'))
}

function Harness({ grouped = false }: { grouped?: boolean }) {
  const [container, setContainer] = useState<HTMLDivElement | null>(null)
  const [quote, setQuote] = useState('')
  useEffect(() => onComposerInsert(setQuote), [])
  return <>
    <div ref={setContainer} data-testid="viewport">
      {grouped ? <MessageGroup groupId="g" messages={[
        { id: 'a', role: 'assistant', content: 'Alpha answer text', group_id: 'g', model: 'model-a', timestamp: 1 },
        { id: 'b', role: 'assistant', content: 'Beta answer text', group_id: 'g', model: 'model-b', timestamp: 1 },
      ]} /> : <div data-message-id="single"><p>Single answer text</p></div>}
    </div>
    <p>Outside the message list</p>
    <output aria-label="Quoted text">{quote}</output>
    <AddSelectionToChat containerEl={container} lang="zh" />
  </>
}

const popup = () => screen.queryByRole('button', { name: '添加到聊天' })
const nextFrame = () => act(async () => {
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
})

afterEach(() => {
  window.getSelection()?.removeAllRanges()
  setMode('tabs')
})

describe('AddSelectionToChat', () => {
  it.each(['tabs', 'columns'] as const)('quotes text from a grouped answer in %s mode', async mode => {
    setMode(mode)
    render(<Harness grouped />)
    if (mode === 'tabs') fireEvent.click(screen.getByRole('button', { name: 'model-b' }))
    const paragraph = screen.getByText('Beta answer text')
    fireEvent.mouseDown(paragraph)
    selectText(paragraph)
    await nextFrame()
    expect(popup()).toBeNull()
    fireEvent.mouseUp(paragraph)
    const button = await screen.findByRole('button', { name: '添加到聊天' })
    fireEvent.mouseDown(button)
    fireEvent.mouseUp(button)
    fireEvent.click(button)
    expect(screen.getByLabelText('Quoted text')).toHaveTextContent('Beta answer text')
    await nextFrame()
    expect(popup()).toBeNull()
  })

  it('shows keyboard selections and quotes the latest extended selection without mouseup', async () => {
    render(<Harness />)
    const paragraph = screen.getByText('Single answer text')
    selectText(paragraph, 0, 6)
    await screen.findByRole('button', { name: '添加到聊天' })
    selectText(paragraph, 7)
    await nextFrame()
    fireEvent.click(screen.getByRole('button', { name: '添加到聊天' }))
    expect(screen.getByLabelText('Quoted text')).toHaveTextContent(/^answer text$/)
  })

  it('shows a selection committed after the mouseup frame has already run', async () => {
    render(<Harness />)
    const paragraph = screen.getByText('Single answer text')
    fireEvent.mouseDown(paragraph)
    fireEvent.mouseUp(paragraph)
    await nextFrame()
    expect(popup()).toBeNull()
    selectText(paragraph)
    await screen.findByRole('button', { name: '添加到聊天' })
  })

  it('hides on scroll and restores when text is selected again', async () => {
    render(<Harness />)
    const paragraph = screen.getByText('Single answer text')
    selectText(paragraph)
    await screen.findByRole('button', { name: '添加到聊天' })
    fireEvent.scroll(screen.getByTestId('viewport'))
    expect(popup()).toBeNull()
    selectText(paragraph, 0, 6)
    await screen.findByRole('button', { name: '添加到聊天' })
  })

  it('rejects selections outside the message list and clears collapsed selections', async () => {
    render(<Harness />)
    selectText(screen.getByText('Single answer text'))
    await screen.findByRole('button', { name: '添加到聊天' })
    selectText(screen.getByText('Outside the message list'))
    await waitFor(() => expect(popup()).toBeNull())
    selectText(screen.getByText('Single answer text'))
    await screen.findByRole('button', { name: '添加到聊天' })
    window.getSelection()!.removeAllRanges()
    fireEvent(document, new Event('selectionchange'))
    await waitFor(() => expect(popup()).toBeNull())
  })
})
