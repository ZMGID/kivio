import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ContextIndicator } from './ContextIndicator'

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

describe('ContextIndicator stop action', () => {
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
