import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ContextIndicator } from './ContextIndicator'
import type { ConversationContextState } from './types'

const settings = vi.hoisted(() => ({
  providers: [
    { id: 'first', modelOverrides: {
      'context-small': { contextWindow: 128_000 },
      'context-large': { contextWindow: 256_000 },
      'context-invalid': { contextWindow: 0 },
    } },
    { id: 'second', modelOverrides: { 'context-small': { contextWindow: 64_000 } } },
  ],
}))

vi.mock('../api/settingsCache', () => ({
  peekSettings: () => settings,
  getSettingsCached: async () => settings,
  subscribeSettings: () => () => {},
}))

describe('selected model context capacity', () => {
  it('shows capacity before usage exists and follows model and provider changes', () => {
    const { rerender } = render(<ContextIndicator model="context-small" providerId="first" lang="en" />)
    fireEvent.click(screen.getByRole('button', { name: 'Context' }))
    expect(screen.getByText('—/128.0k')).toBeVisible()
    rerender(<ContextIndicator model="context-large" providerId="first" lang="en" />)
    expect(screen.getByText('—/256.0k')).toBeVisible()
    rerender(<ContextIndicator model="context-small" providerId="second" lang="en" />)
    expect(screen.getByText('—/64.0k')).toBeVisible()
    rerender(<ContextIndicator model="unknown-context-model" providerId="first" lang="en" />)
    expect(screen.getByText('—')).toBeVisible()
    rerender(<ContextIndicator model="context-invalid" providerId="first" lang="en" />)
    expect(screen.getByText('—')).toBeVisible()
  })

  it('keeps reported runtime capacity ahead of selected model metadata', () => {
    render(<ContextIndicator model="context-small" providerId="first" lang="en"
      contextState={{ reported_context_tokens: 32_000, context_window_tokens: 64_000, token_count_source: 'provider_context_reported' }} />)
    fireEvent.click(screen.getByRole('button', { name: 'Context' }))
    expect(screen.getByText('32.0k/64.0k (50%)')).toBeVisible()
  })

  it('does not use a Kivio provider override for an external CLI', () => {
    render(<ContextIndicator model="context-small" providerId="first" usesExternalRuntime lang="en" />)
    fireEvent.click(screen.getByRole('button', { name: 'Context' }))
    expect(screen.getByText('—')).toBeVisible()
  })
})

const breakdownState: ConversationContextState = {
  reported_context_tokens: 53_000,
  token_count_source: 'provider_context_reported',
  context_window_tokens: 1_000_000,
  segments: [
    { id: 'system_prompt', label: '', chars: 200, estimated_tokens: 1_000 },
    { id: 'tools', label: '', chars: 500, estimated_tokens: 4_000 },
    { id: 'conversation', label: '', chars: 300, estimated_tokens: 2_000 },
  ],
}

function openPanel(props: Partial<Parameters<typeof ContextIndicator>[0]>) {
  const onStopCompression = vi.fn()
  render(
    <ContextIndicator
      contextState={null}
      messageCount={4}
      compressing
      onCompress={vi.fn()}
      onStopCompression={onStopCompression}
      lang="en"
      {...props}
    />,
  )
  fireEvent.click(screen.getByLabelText('Context'))
  return onStopCompression
}

describe('ContextIndicator character share', () => {
  it('labels UTF-16 shares beside three category estimates and the k capacity', () => {
    render(
      <ContextIndicator
        contextState={breakdownState}
        messageCount={4}
        lang="en"
        onRefresh={vi.fn()}
        onCompress={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByLabelText('Context'))
    expect(screen.getByText('53.0k/1000.0k (5.3%)')).toBeTruthy()
    expect(screen.getByText('Character share')).toBeTruthy()
    expect(screen.getByText('System prompt')).toBeTruthy()
    expect(screen.getByText('Tools')).toBeTruthy()
    expect(screen.getByText('Conversation')).toBeTruthy()
    expect(screen.getByText('≈ 4.0k')).toBeTruthy()
    expect(screen.getByText('≈ 2.0k')).toBeTruthy()
    expect(screen.getByText('≈ 1.0k')).toBeTruthy()
    expect(screen.getByText('50%')).toBeTruthy()
    expect(screen.getByText('30%')).toBeTruthy()
    expect(screen.getByText('20%')).toBeTruthy()
    expect(screen.getByTitle(/UTF-16 character shares, not token shares/)).toBeTruthy()
  })

})

describe('ContextIndicator usage panel', () => {

  it('lets keyboard users scroll details and returns focus on Escape', () => {
    render(<ContextIndicator lang="en" />)
    const trigger = screen.getByRole('button', { name: 'Context' })
    trigger.focus()
    fireEvent.click(trigger)
    expect(screen.getByRole('button', { name: 'Refresh context' })).toHaveFocus()
    trigger.focus()
    fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    expect(screen.getByRole('button', { name: 'Refresh context' })).toHaveFocus()
    const details = screen.getByRole('region', { name: 'Context capacity' })
    expect(details).toHaveAttribute('tabindex', '0')
    details.focus()
    const down = new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })
    fireEvent(details, down)
    expect(down.defaultPrevented).toBe(false)
    expect(details).toHaveFocus()
    fireEvent.keyDown(details, { key: 'Escape' })
    expect(trigger).toHaveFocus()
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('region', { name: 'Context capacity' })).not.toBeInTheDocument()
  })

  it('keeps context actions outside the scrollable details and preserves their behavior', () => {
    const onRefresh = vi.fn()
    const onCompress = vi.fn()
    const onClear = vi.fn()
    render(
      <ContextIndicator
        lang="en"
        messageCount={4}
        lastMessageId="last"
        onRefresh={onRefresh}
        onCompress={onCompress}
        onClear={onClear}
      />,
    )
    const trigger = screen.getByRole('button', { name: 'Context' })
    fireEvent.click(trigger)
    const details = screen.getByRole('region', { name: 'Context capacity' })
    const refresh = screen.getByRole('button', { name: 'Refresh context' })
    const compress = screen.getByRole('button', { name: 'Compress context' })
    const clear = screen.getByRole('button', { name: 'Clear context' })
    expect(details).not.toContainElement(refresh)
    expect(details).not.toContainElement(compress)
    expect(details).not.toContainElement(clear)
    fireEvent.click(refresh)
    fireEvent.click(compress)
    fireEvent.click(clear)
    expect(onRefresh).toHaveBeenCalledTimes(1)
    expect(onCompress).toHaveBeenCalledTimes(1)
    expect(onClear).toHaveBeenCalledTimes(1)
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
  })
})

describe('ContextIndicator stop action', () => {
  it('explains Antigravity automatic management without offering unsupported manual compaction', () => {
    const onCompress = vi.fn()
    openPanel({ compressing: false, usesExternalRuntime: true,
      contextState: { external_agent_id: 'antigravity' }, onCompress })
    const action = screen.getByRole('button', { name: 'CLI manages context' })
    expect(action).toBeDisabled()
    fireEvent.click(action)
    expect(onCompress).not.toHaveBeenCalled()
    expect(action.title).toContain('manual compaction')
  })

  it('offers to stop a manual compaction', () => {
    const onStop = openPanel({ generating: false })
    fireEvent.click(screen.getByRole('button', { name: 'Stop compaction' }))
    expect(onStop).toHaveBeenCalledTimes(1)
  })

  it('says it stops the generation during automatic compaction', () => {
    // Automatic compaction runs inside a generation; stopping it stops that generation.
    openPanel({ generating: true })
    expect(screen.getByRole('button', { name: 'Stop generating' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Stop compaction' })).toBeNull()
  })
})

it('keeps usage details inside a narrow viewport when the trigger is inset', () => {
  vi.stubGlobal('innerWidth', 360)
  try {
    render(<ContextIndicator lang="en" />)
    const trigger = screen.getByRole('button', { name: 'Context' })
    vi.spyOn(trigger, 'getBoundingClientRect').mockReturnValue({
      x: 250, y: 350, left: 250, right: 275, top: 350, bottom: 375,
      width: 25, height: 25, toJSON: () => ({}),
    })
    fireEvent.click(trigger)
    const panel = document.getElementById(trigger.getAttribute('aria-controls')!)!
    const right = parseFloat(panel.style.right)
    const width = parseFloat(panel.style.width)
    expect(360 - right - width).toBeGreaterThanOrEqual(8)
    expect(right).toBeGreaterThanOrEqual(8)
  } finally {
    vi.unstubAllGlobals()
  }
})
