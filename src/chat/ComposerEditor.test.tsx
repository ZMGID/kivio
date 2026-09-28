import { createRef, useState } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { ComposerEditor, type ComposerEditorHandle } from './ComposerEditor'
import type { SlashCommandDefinition } from './slashCommands'

const commands: SlashCommandDefinition[] = [
  { id: 'plan', slash: '/plan', title: 'Plan', description: 'Plan a task', category: 'Actions', keywords: [], kind: 'action' },
  { id: 'skill:review', slash: '/review', title: 'Review', description: 'Review code', category: 'Skills', keywords: [], kind: 'skill' },
  { id: 'cli:claude:compact', slash: '/compact', title: 'Compact', description: 'Compact context', category: 'Claude', keywords: [], kind: 'cli', agentId: 'claude' },
]

beforeAll(() => {
  // jsdom has selection but no layout geometry for editor scrollIntoView.
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})

function setup(initial = '') {
  const editor = createRef<ComposerEditorHandle>()
  const change = vi.fn()
  function Harness() {
    const [value, setValue] = useState(initial)
    return <ComposerEditor ref={editor} value={value} scopeKey="a" commands={commands}
      placeholder="Write" onChange={(next) => { change(next); setValue(next) }}
      onSelect={() => {}} onKeyDown={() => {}} onPaste={() => {}} onContextMenu={() => {}} />
  }
  const result = render(<Harness />)
  return { ...result, editor, change, textbox: screen.getByRole('textbox') }
}

describe('ComposerEditor', () => {
  it('renders every command kind inline with icons and preserves plain-text serialization', () => {
    const text = '先 /plan 然后请用/review 最后 /compact '
    const { editor, textbox } = setup(text)
    expect(textbox.querySelectorAll('[data-command]')).toHaveLength(3)
    expect(textbox.querySelector('[data-command="plan"] svg')).not.toBeNull()
    expect(textbox.querySelector('[data-command="cli:claude:compact"] img')).not.toBeNull()
    expect(editor.current!.value).toBe(text)
  })

  it('inserts a command in the middle, preserves surrounding text and places the caret after it', () => {
    const { editor, textbox } = setup('请用/rev 检查这里')
    act(() => editor.current!.replaceText(2, 6, '/review '))
    expect(editor.current!.value).toBe('请用/review  检查这里')
    expect(editor.current!.selectionStart).toBe(10)
    expect(textbox.querySelectorAll('[data-command]')).toHaveLength(1)
    act(() => editor.current!.undo())
    expect(editor.current!.value).toBe('请用/rev 检查这里')
    act(() => editor.current!.redo())
    expect(editor.current!.value).toBe('请用/review  检查这里')
  })

  it('snaps partial selections to command boundaries and deletes/restores a whole chip', () => {
    const { editor } = setup('请用/review 检查')
    act(() => {
      editor.current!.setSelectionRange(3, 6)
      editor.current!.replaceText(editor.current!.selectionStart, editor.current!.selectionEnd, '')
    })
    expect(editor.current!.value).toBe('请用 检查')
    act(() => editor.current!.undo())
    expect(editor.current!.value).toBe('请用/review 检查')
  })

  it('pastes plain text, preserves newlines and ignores pasted HTML', () => {
    const { editor, textbox } = setup()
    fireEvent.paste(textbox, { clipboardData: { getData: (type: string) => type === 'text/plain' ? '第一行\n/review 任务' : '<b>bad</b>', files: [] } })
    expect(editor.current!.value).toBe('第一行\n/review 任务')
    expect(textbox.querySelectorAll('[data-command]')).toHaveLength(1)
    expect(textbox.querySelector('b')).toBeNull()
  })

  it('does not turn an exact-but-unfinished query into an atom while typing', () => {
    const { textbox, editor } = setup()
    fireEvent.paste(textbox, { clipboardData: { getData: () => '/plan', files: [] } })
    expect(editor.current!.value).toBe('/plan')
    expect(textbox.querySelector('[data-command]')).toBeNull()
    fireEvent.paste(textbox, { clipboardData: { getData: () => ' ', files: [] } })
    expect(textbox.querySelector('[data-command="plan"]')).not.toBeNull()
  })

  it('preserves newlines when browser typing reparses the text DOM', async () => {
    const { editor, textbox } = setup('第一行\n')
    const text = textbox.firstChild as Text
    text.appendData('第二行')
    await waitFor(() => expect(editor.current!.value).toBe('第一行\n第二行'))
  })

  it('keeps read-only editing locked and resets history between drafts', () => {
    const editor = createRef<ComposerEditorHandle>()
    const props = { ref: editor, commands, placeholder: 'Write', onChange: vi.fn(), onSelect: vi.fn(), onKeyDown: vi.fn(), onPaste: vi.fn(), onContextMenu: vi.fn() }
    const { rerender } = render(<ComposerEditor {...props} scopeKey="a" value="/plan " />)
    act(() => editor.current!.replaceText(6, 6, 'hello'))
    rerender(<ComposerEditor {...props} scopeKey="b" value="other draft" readOnly />)
    expect(editor.current!.canUndo).toBe(false)
    act(() => editor.current!.replaceText(0, 5, 'changed'))
    expect(editor.current!.value).toBe('other draft')
    expect(screen.getByRole('textbox')).toHaveAttribute('contenteditable', 'false')
  })

  it('refreshes an existing chip icon when its command metadata changes without changing the draft', () => {
    const editor = createRef<ComposerEditorHandle>()
    const props = { ref: editor, placeholder: 'Write', onChange: vi.fn(), onSelect: vi.fn(), onKeyDown: vi.fn(), onPaste: vi.fn(), onContextMenu: vi.fn(), scopeKey: 'a', value: '/review ' }
    const fallback = { ...commands[1], kind: 'action' as const }
    const { rerender } = render(<ComposerEditor {...props} commands={[fallback]} />)
    const textbox = screen.getByRole('textbox')
    expect(textbox.querySelector('.lucide-sparkles')).not.toBeNull()
    act(() => editor.current!.setSelectionRange(8, 8))
    rerender(<ComposerEditor {...props} commands={[commands[1]]} />)
    expect(textbox.querySelector('.lucide-sparkles')).toBeNull()
    expect(textbox.querySelector('.chat-composer-command-icon svg')).not.toBeNull()
    expect(editor.current!.value).toBe('/review ')
    expect(editor.current!.selectionStart).toBe(8)
  })
})
