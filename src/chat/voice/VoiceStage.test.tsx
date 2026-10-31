import { useState } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { VoiceStage } from './VoiceStage'
import { VoiceTrigger } from './VoiceTrigger'
import { VoiceCallContext, type VoiceCallApi, type VoiceState, type VoiceTranscriptRow } from './useVoiceCall'

vi.mock('./VoiceOrb', () => ({ VoiceOrb: () => null }))

function StageHarness({ state = 'active', muting = false, toggleMuted = vi.fn(), realTrigger = false, transcriptRows = [] }: {
  state?: VoiceState
  muting?: boolean
  toggleMuted?: () => void
  realTrigger?: boolean
  transcriptRows?: VoiceTranscriptRow[]
}) {
  const [stageOpen, setStageOpen] = useState(false)
  const [callState, setCallState] = useState(state)
  const call: VoiceCallApi = {
    state: callState, muting, muted: false, working: 0,
    transcriptRows,
    conversationId: null, channel: 'codex', activeSince: null, lastError: null,
    stageOpen, setStageOpen,
    levelsRef: { current: { mic: 0, speaker: 0 } },
    audioRef: { current: null },
    start: () => { setCallState('connecting'); setStageOpen(true) }, hangup: vi.fn(), toggleMuted,
  }
  return <>
    {!realTrigger && <button onClick={() => setStageOpen(true)}>Open voice</button>}
    <VoiceCallContext.Provider value={call}>
      {realTrigger && <VoiceTrigger lang="en" />}
      <VoiceStage lang="en" sidebarOffset="0px" onOpenConversation={vi.fn()} />
    </VoiceCallContext.Provider>
  </>
}

describe('voice stage accessibility', () => {
  it('moves focus into the stage and returns it to the opener on Escape', () => {
    render(<StageHarness />)
    const opener = screen.getByRole('button', { name: 'Open voice' })
    opener.focus()
    fireEvent.click(opener)
    const stage = screen.getByRole('region', { name: 'Voice orchestrator' })
    expect(stage).toHaveFocus()
    expect(screen.getByRole('group', { name: 'Voice orchestrator' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Mute · M' })).toHaveAttribute('aria-pressed', 'false')
    fireEvent.keyDown(stage, { key: 'Escape' })
    expect(screen.queryByRole('region', { name: 'Voice orchestrator' })).toBeNull()
    expect(opener).toHaveFocus()
  })

  it('keeps the sidebar opener mounted across call start so focus can return', () => {
    render(<StageHarness state="idle" realTrigger />)
    const opener = screen.getByRole('button', { name: 'Start voice · orchestrator' })
    opener.focus()
    fireEvent.click(opener)
    expect(screen.getByRole('button', { name: 'Hide voice · Esc' })).toBe(opener)
    fireEvent.keyDown(screen.getByRole('region', { name: 'Voice orchestrator' }), { key: 'Escape' })
    expect(screen.getByRole('button', { name: 'Open voice' })).toBe(opener)
    expect(opener).toHaveFocus()
  })

  it.each([{ state: 'connecting' as const, muting: false }, { state: 'active' as const, muting: true }])(
    'does not expose mute while $state with muting=$muting',
    ({ state, muting }) => {
      const toggleMuted = vi.fn()
      render(<StageHarness state={state} muting={muting} toggleMuted={toggleMuted} />)
      fireEvent.click(screen.getByRole('button', { name: 'Open voice' }))
      const stage = screen.getByRole('region', { name: 'Voice orchestrator' })
      expect(screen.getByRole('button', { name: 'Mute · M' })).toBeDisabled()
      fireEvent.keyDown(stage, { key: 'm' })
      expect(toggleMuted).not.toHaveBeenCalled()
    },
  )

  it('shows only the current speaker and does not let a late previous turn replace it', () => {
    const view = render(<StageHarness transcriptRows={[
      { id: 'u', role: 'user', text: '请帮我整理计划。', finalized: false },
    ]} />)
    fireEvent.click(screen.getByRole('button', { name: 'Open voice' }))
    expect(screen.getByRole('status', { name: 'Live captions' })).toHaveTextContent('请帮我整理计划。')
    view.rerender(<StageHarness transcriptRows={[
      { id: 'u', role: 'user', text: '请帮我整理计划。', finalized: true },
      { id: 'a', role: 'assistant', text: '先确认', finalized: false },
    ]} />)
    expect(screen.queryByText('请帮我整理计划。')).toBeNull()
    expect(screen.getByRole('status', { name: 'Live captions' })).toHaveTextContent('先确认')
    view.rerender(<StageHarness transcriptRows={[
      { id: 'u', role: 'user', text: '请帮我整理明天的计划。', finalized: true },
      { id: 'a', role: 'assistant', text: '先确认交付目标，再拆解任务。', finalized: true },
    ]} />)
    expect(screen.getByRole('status', { name: 'Live captions' })).toHaveTextContent('先确认交付目标，再拆解任务。')
    expect(screen.queryByText('请帮我整理明天的计划。')).toBeNull()
  })

  it('lets the call bar tuck away the stage and reopen the same live call', () => {
    render(<StageHarness realTrigger />)
    fireEvent.click(screen.getByRole('button', { name: 'Open voice' }))
    fireEvent.click(screen.getByRole('button', { name: 'Back to chats · Esc' }))
    expect(screen.queryByRole('region', { name: 'Voice orchestrator' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Open voice' }))
    expect(screen.getByRole('group', { name: 'Voice orchestrator' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Mute · M' })).toBeEnabled()
  })
})
