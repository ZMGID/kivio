import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import type { VoiceCodexEvent } from '../../api/tauri'
import { useVoiceCall } from './useVoiceCall'

const mocks = vi.hoisted(() => ({
  settings: vi.fn(),
  listen: vi.fn(),
  reserve: vi.fn(),
  start: vi.fn(),
  stop: vi.fn(),
  mute: vi.fn(),
  liveStart: vi.fn(),
  delegate: vi.fn(),
  notify: vi.fn(),
}))

vi.mock('../../api/tauri', () => ({ api: {
  onVoiceCodexEvent: mocks.listen,
  voiceCodexReserve: mocks.reserve,
  voiceCodexStart: mocks.start,
  voiceCodexStop: mocks.stop,
  voiceCodexMute: mocks.mute,
  voiceLiveStart: mocks.liveStart,
  voiceDelegate: mocks.delegate,
} }))
vi.mock('../../api/settingsCache', () => ({ getSettingsCached: mocks.settings }))
vi.mock('../../components/appNoticeStore', () => ({ notifyError: mocks.notify }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

let listeners: ((event: VoiceCodexEvent) => void)[]
let unlisteners: Mock[]
let getUserMedia: Mock
const originalMediaDevices = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices')
const onDelegationComplete = vi.fn()

function callHook() {
  return renderHook(() => useVoiceCall({ providerId: 'provider', model: 'model', lang: 'en', onDelegationComplete }))
}

function emit(index: number, event: VoiceCodexEvent) {
  act(() => listeners[index](event))
}

beforeEach(() => {
  vi.resetAllMocks()
  listeners = []
  unlisteners = []
  getUserMedia = vi.fn()
  Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })
  mocks.settings.mockResolvedValue({ voice: { channel: 'auto' } })
  mocks.listen.mockImplementation(async (listener: (event: VoiceCodexEvent) => void) => {
    listeners.push(listener)
    const unlisten = vi.fn()
    unlisteners.push(unlisten)
    return unlisten
  })
  mocks.reserve.mockResolvedValue('session')
  mocks.start.mockResolvedValue(undefined)
  mocks.stop.mockResolvedValue(undefined)
  mocks.mute.mockResolvedValue(undefined)
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  if (originalMediaDevices) Object.defineProperty(navigator, 'mediaDevices', originalMediaDevices)
  else Reflect.deleteProperty(navigator, 'mediaDevices')
})

describe('Codex voice integration', () => {
  it('reconciles item identities, unknown roles, canonical finals and late events', async () => {
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'session', type: 'final', role: 'user', itemId: 'user-1', text: 'First question' })
    emit(0, { sessionId: 'session', type: 'final', role: 'assistant', itemId: 'answer-1', text: 'First answer' })

    emit(0, { sessionId: 'session', type: 'partial', itemId: 'answer-2', delta: 'Another ' })
    emit(0, { sessionId: 'session', type: 'partial', itemId: 'answer-2', delta: 'answer' })
    const unknownId = result.current.transcriptRows[2].id
    expect(result.current.transcriptRows[2]).toMatchObject({ role: 'unknown', text: 'Another answer', finalized: false })
    emit(0, { sessionId: 'session', type: 'final', role: 'user', itemId: 'user-1', text: 'First question corrected' })
    emit(0, { sessionId: 'session', type: 'final', role: 'assistant', itemId: 'answer-2', text: 'Another answer.' })
    emit(0, { sessionId: 'session', type: 'partial', itemId: 'answer-2', delta: ' stale' })
    expect(result.current.transcriptRows[2]).toEqual({ id: unknownId, role: 'assistant', text: 'Another answer.', finalized: true })

    emit(0, { sessionId: 'session', type: 'partial', itemId: 'answer-3', delta: 'On it' })
    emit(0, { sessionId: 'session', type: 'final', role: 'user', itemId: 'user-2', text: 'Open the test' })
    emit(0, { sessionId: 'session', type: 'partial', itemId: 'answer-3', delta: ' now' })
    expect(result.current.transcriptRows.map(({ role, text, finalized }) => ({ role, text, finalized }))).toEqual([
      { role: 'user', text: 'First question', finalized: true },
      { role: 'assistant', text: 'First answer', finalized: true },
      { role: 'assistant', text: 'Another answer.', finalized: true },
      { role: 'user', text: 'Open the test', finalized: true },
      { role: 'unknown', text: 'On it now', finalized: false },
    ])
  })

  it('reconciles anonymous partials with their final and starts a new anonymous turn afterwards', async () => {
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'session', type: 'partial', delta: 'Hello ' })
    emit(0, { sessionId: 'session', type: 'partial', delta: 'there' })
    emit(0, { sessionId: 'session', type: 'final', role: 'assistant', itemId: 'answer', text: 'Hello there.' })
    expect(result.current.transcriptRows).toEqual([{ id: 'voice-1', role: 'assistant', text: 'Hello there.', finalized: true }])
    emit(0, { sessionId: 'session', type: 'partial', delta: 'Next' })
    expect(result.current.transcriptRows[1]).toMatchObject({ role: 'unknown', text: 'Next', finalized: false })
  })

  it('shows the normalized Codex 0.164 flat transcript events for an ephemeral thread', async () => {
    const { result } = callHook()
    await act(async () => { result.current.start() })
    // These are the normalized outputs of the upstream v3 transcript fixture exercised by the Rust test.
    emit(0, { sessionId: 'session', type: 'partial', role: 'user', itemId: 'flat-1', delta: 'What can you do?' })
    emit(0, { sessionId: 'session', type: 'partial', role: 'assistant', itemId: 'flat-2', delta: 'A bunch ' })
    emit(0, { sessionId: 'session', type: 'partial', role: 'assistant', itemId: 'flat-2', delta: 'of stuff' })
    expect(result.current.transcriptRows.map(({ role, text, finalized }) => ({ role, text, finalized }))).toEqual([
      { role: 'user', text: 'What can you do?', finalized: false },
      { role: 'assistant', text: 'A bunch of stuff', finalized: false },
    ])
    emit(0, { sessionId: 'session', type: 'final', role: 'user', itemId: 'flat-1', text: 'What can you do?', preserveLonger: true })
    emit(0, { sessionId: 'session', type: 'final', role: 'assistant', itemId: 'flat-2', text: 'A bunch of stuff, including coding.', preserveLonger: true })
    emit(0, { sessionId: 'session', type: 'final', role: 'user', itemId: 'flat-3', text: 'One more question', preserveLonger: true })
    expect(result.current.transcriptRows.map(({ role, text, finalized }) => ({ role, text, finalized }))).toEqual([
      { role: 'user', text: 'What can you do?', finalized: true },
      { role: 'assistant', text: 'A bunch of stuff, including coding.', finalized: true },
      { role: 'user', text: 'One more question', finalized: true },
    ])
  })

  it('preserves newer flat speech when an older final arrives', async () => {
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'session', type: 'partial', role: 'assistant', itemId: 'flat-1', delta: 'Okay.' })
    emit(0, { sessionId: 'session', type: 'partial', role: 'assistant', itemId: 'flat-1', delta: 'I will check' })
    emit(0, { sessionId: 'session', type: 'final', role: 'assistant', itemId: 'flat-1', text: 'Okay.', preserveLonger: true })
    expect(result.current.transcriptRows[0]).toMatchObject({ role: 'assistant', text: 'Okay.I will check', finalized: true })
  })

  it('bounds recent turns and ignores late events for evicted completed items', async () => {
    const { result } = callHook()
    await act(async () => { result.current.start() })
    act(() => {
      for (let i = 0; i < 35; i += 1) {
        listeners[0]({ sessionId: 'session', type: 'final', role: 'user', itemId: `item-${i}`, text: `${i}` })
      }
    })
    expect(result.current.transcriptRows).toHaveLength(32)
    expect(result.current.transcriptRows[0].text).toBe('3')
    emit(0, { sessionId: 'session', type: 'partial', itemId: 'item-0', delta: 'late' })
    emit(0, { sessionId: 'session', type: 'final', role: 'user', itemId: 'item-0', text: 'duplicate' })
    expect(result.current.transcriptRows).toHaveLength(32)
    expect(result.current.transcriptRows[0].text).toBe('3')
    emit(0, { sessionId: 'session', type: 'partial', itemId: 'long', delta: '😀'.repeat(4100) })
    expect(Array.from(result.current.transcriptRows[31].text)).toHaveLength(4096)
  })

  it('buffers early events and filters other sessions before the start returns', async () => {
    const starting = deferred<void>()
    mocks.start.mockReturnValueOnce(starting.promise)
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'other', type: 'final', role: 'user', itemId: 'wrong', text: 'Wrong' })
    emit(0, { sessionId: 'session', type: 'active' })
    emit(0, { sessionId: 'session', type: 'partial', itemId: 'answer', delta: 'Early' })
    await act(async () => { starting.resolve() })
    expect(result.current.state).toBe('active')
    expect(result.current.channel).toBe('codex')
    expect(result.current.transcriptRows).toEqual([{ id: 'voice-1', role: 'unknown', text: 'Early', finalized: false }])
    emit(0, { sessionId: 'other', type: 'final', role: 'user', itemId: 'wrong', text: 'Wrong' })
    expect(result.current.transcriptRows).toHaveLength(1)
    expect(getUserMedia).not.toHaveBeenCalled()
  })

  it('unsubscribes a late subscription without starting or disturbing a replacement call', async () => {
    const subscribing = deferred<() => void>()
    const oldUnlisten = vi.fn()
    mocks.listen.mockImplementationOnce((listener: (event: VoiceCodexEvent) => void) => {
      listeners.push(listener)
      return subscribing.promise
    })
    const { result } = callHook()
    await act(async () => { result.current.start() })
    act(() => result.current.hangup())
    await act(async () => { result.current.start() })
    emit(1, { sessionId: 'session', type: 'active' })
    await act(async () => { subscribing.resolve(oldUnlisten) })
    expect(oldUnlisten).toHaveBeenCalledOnce()
    expect(mocks.start).toHaveBeenCalledTimes(1)
    expect(result.current.state).toBe('active')
    expect(unlisteners[0]).not.toHaveBeenCalled()
  })

  it('cancels a connecting native start immediately, before the start promise resolves', async () => {
    const starting = deferred<void>()
    const stopping = deferred<void>()
    mocks.start.mockReturnValueOnce(starting.promise)
    mocks.stop.mockReturnValueOnce(stopping.promise)
    const { result } = callHook()
    await act(async () => { result.current.start() })
    expect(mocks.start).toHaveBeenCalledWith('session')
    expect(result.current.state).toBe('connecting')
    act(() => result.current.hangup())
    expect(mocks.stop).toHaveBeenCalledWith('session')
    expect(result.current.state).toBe('closing')
    await act(async () => { stopping.resolve() })
    expect(result.current.state).toBe('idle')
    await act(async () => { starting.reject('Codex voice session cancelled') })
    expect(result.current.state).toBe('idle')
    expect(getUserMedia).not.toHaveBeenCalled()
    expect(mocks.notify).not.toHaveBeenCalled()
  })

  it('does not start a reservation that arrives after hangup and a replacement', async () => {
    const reserving = deferred<string>()
    mocks.reserve.mockReturnValueOnce(reserving.promise).mockResolvedValueOnce('new')
    const { result } = callHook()
    await act(async () => { result.current.start() })
    act(() => result.current.hangup())
    expect(result.current.state).toBe('idle')
    await act(async () => { result.current.start() })
    emit(1, { sessionId: 'new', type: 'active' })
    await act(async () => { reserving.resolve('old') })
    expect(mocks.start).toHaveBeenCalledTimes(1)
    expect(mocks.start).toHaveBeenCalledWith('new')
    expect(mocks.stop).toHaveBeenCalledWith('old')
    expect(mocks.stop).not.toHaveBeenCalledWith('new')
    expect(result.current.state).toBe('active')
    expect(mocks.notify).not.toHaveBeenCalled()
  })

  it('releases a reservation that arrives after unmount without starting it', async () => {
    const reserving = deferred<string>()
    mocks.reserve.mockReturnValueOnce(reserving.promise)
    const { result, unmount } = callHook()
    await act(async () => { result.current.start() })
    unmount()
    await act(async () => { reserving.resolve('late') })
    expect(mocks.start).not.toHaveBeenCalled()
    expect(mocks.stop).toHaveBeenCalledWith('late')
    expect(unlisteners[0]).toHaveBeenCalledOnce()
  })

  it('cancels a connecting native start when the hook unmounts', async () => {
    const starting = deferred<void>()
    mocks.start.mockReturnValueOnce(starting.promise)
    const { result, unmount } = callHook()
    await act(async () => { result.current.start() })
    unmount()
    expect(mocks.stop).toHaveBeenCalledWith('session')
    expect(unlisteners[0]).toHaveBeenCalledOnce()
    await act(async () => { starting.reject('Codex voice session cancelled') })
    expect(mocks.notify).not.toHaveBeenCalled()
  })

  it('reports a connecting cleanup failure without falling back to another device', async () => {
    const starting = deferred<void>()
    mocks.start.mockReturnValueOnce(starting.promise)
    mocks.stop.mockRejectedValueOnce(new Error('cleanup failed'))
    const { result } = callHook()
    await act(async () => { result.current.start() })
    await act(async () => { result.current.hangup() })
    expect(result.current.state).toBe('idle')
    expect(result.current.lastError).toContain('cleanup failed')
    await act(async () => { starting.reject('unavailable: late failure') })
    expect(getUserMedia).not.toHaveBeenCalled()
    expect(mocks.notify).toHaveBeenCalledTimes(1)
  })


  it('stops only a cancelled late start and ignores its events after a replacement starts', async () => {
    const starting = deferred<void>()
    mocks.reserve.mockResolvedValueOnce('old').mockResolvedValueOnce('new')
    mocks.start.mockReturnValueOnce(starting.promise).mockResolvedValueOnce(undefined)
    const { result } = callHook()
    await act(async () => { result.current.start() })
    await act(async () => { result.current.hangup() })
    expect(unlisteners[0]).toHaveBeenCalledOnce()
    await act(async () => { result.current.start() })
    emit(1, { sessionId: 'new', type: 'active' })
    emit(1, { sessionId: 'new', type: 'final', role: 'user', itemId: 'new-item', text: 'Current' })
    emit(0, { sessionId: 'old', type: 'final', role: 'user', itemId: 'old-item', text: 'Stale' })
    emit(0, { sessionId: 'old', type: 'closed', error: 'stale' })
    await act(async () => { starting.resolve() })
    expect(mocks.stop).toHaveBeenCalledWith('old')
    expect(mocks.stop).not.toHaveBeenCalledWith('new')
    expect(result.current.state).toBe('active')
    expect(result.current.transcriptRows).toEqual([{ id: 'voice-1', role: 'user', text: 'Current', finalized: true }])
    expect(mocks.notify).not.toHaveBeenCalled()
  })

  it('does not report a stale stop failure or dispose the replacement call', async () => {
    const stopping = deferred<void>()
    mocks.stop.mockReturnValueOnce(stopping.promise)
    mocks.reserve.mockResolvedValueOnce('old').mockResolvedValueOnce('new')
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'old', type: 'active' })
    act(() => result.current.hangup())
    emit(0, { sessionId: 'old', type: 'active' })
    expect(result.current.state).toBe('closing')
    emit(0, { sessionId: 'old', type: 'closed' })
    await act(async () => { result.current.start() })
    emit(1, { sessionId: 'new', type: 'active' })
    await act(async () => { stopping.reject(new Error('late stop failure')) })
    expect(result.current.state).toBe('active')
    expect(result.current.lastError).toBeNull()
    expect(mocks.notify).not.toHaveBeenCalled()
  })

  it('reports a cleanup error delivered in the closing event even if stop rejects later', async () => {
    const stopping = deferred<void>()
    mocks.stop.mockReturnValueOnce(stopping.promise)
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'session', type: 'active' })
    act(() => result.current.hangup())
    emit(0, { sessionId: 'session', type: 'closed', error: 'cleanup failed' })
    expect(result.current.lastError).toContain('cleanup failed')
    await act(async () => { stopping.reject(new Error('cleanup failed')) })
    expect(mocks.notify).toHaveBeenCalledTimes(1)
  })

  it('shows only acknowledged mute state and prevents overlapping commands', async () => {
    const first = deferred<void>()
    mocks.mute.mockReturnValueOnce(first.promise).mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('unmute failed'))
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'session', type: 'active' })
    act(() => { result.current.toggleMuted(); result.current.toggleMuted() })
    expect(result.current.muting).toBe(true)
    expect(result.current.muted).toBe(false)
    expect(mocks.mute).toHaveBeenCalledTimes(1)
    await act(async () => { first.reject(new Error('mute failed')) })
    expect(result.current.muting).toBe(false)
    expect(result.current.muted).toBe(false)
    await act(async () => { result.current.toggleMuted() })
    expect(result.current.muted).toBe(true)
    await act(async () => { result.current.toggleMuted() })
    expect(result.current.muted).toBe(true)
    expect(mocks.mute).toHaveBeenLastCalledWith('session', false)
    expect(mocks.notify).toHaveBeenCalledTimes(2)
  })

  it('ignores a late mute acknowledgement after hangup and restart', async () => {
    const muting = deferred<void>()
    mocks.mute.mockReturnValueOnce(muting.promise)
    mocks.reserve.mockResolvedValueOnce('old').mockResolvedValueOnce('new')
    const { result } = callHook()
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'old', type: 'active' })
    act(() => result.current.toggleMuted())
    expect(result.current.muting).toBe(true)
    await act(async () => { result.current.hangup() })
    await act(async () => { result.current.start() })
    emit(1, { sessionId: 'new', type: 'active' })
    await act(async () => { muting.resolve() })
    expect(result.current).toMatchObject({ state: 'active', muting: false, muted: false })
    expect(mocks.notify).not.toHaveBeenCalled()
  })

  it('unsubscribes on unmount even if the native stop is still pending', async () => {
    const stopping = deferred<void>()
    mocks.stop.mockReturnValueOnce(stopping.promise)
    const { result, unmount } = callHook()
    await act(async () => { result.current.start() })
    unmount()
    expect(mocks.stop).toHaveBeenCalledWith('session')
    expect(unlisteners[0]).toHaveBeenCalledOnce()
  })
})

describe('GPT-Live selection and microphone lifetime', () => {
  it('falls back only for an unavailable Codex channel in auto mode', async () => {
    const media = deferred<MediaStream>()
    getUserMedia.mockReturnValue(media.promise)
    mocks.start.mockRejectedValueOnce('unavailable: no subscription')
    const { result } = callHook()
    await act(async () => { result.current.start() })
    expect(result.current.channel).toBe('live')
    expect(getUserMedia).toHaveBeenCalledOnce()
    expect(unlisteners[0]).toHaveBeenCalledOnce()
    act(() => result.current.hangup())
  })

  it('does not fall back if cleanup after an unavailable start fails', async () => {
    mocks.start.mockRejectedValueOnce('unavailable: no subscription')
    mocks.stop.mockRejectedValueOnce(new Error('cleanup failed'))
    const { result } = callHook()
    await act(async () => { result.current.start() })
    expect(result.current.state).toBe('error')
    expect(result.current.lastError).toContain('cleanup failed')
    expect(getUserMedia).not.toHaveBeenCalled()
  })

  it.each([
    ['auto', 'network error'],
    ['codex', 'unavailable: no subscription'],
  ])('does not fall back for %s with %s', async (channel, error) => {
    mocks.settings.mockResolvedValue({ voice: { channel } })
    mocks.start.mockRejectedValueOnce(error)
    const { result } = callHook()
    await act(async () => { result.current.start() })
    expect(result.current.state).toBe('error')
    expect(getUserMedia).not.toHaveBeenCalled()
    expect(unlisteners[0]).toHaveBeenCalledOnce()
  })

  it('keeps GPT-Live audio, transcripts, delegation and hangup on their existing path', async () => {
    const track = { enabled: true, stop: vi.fn() }
    const stream = { getTracks: () => [track], getAudioTracks: () => [track] }
    const dataChannel = {
      readyState: 'open',
      onmessage: null as ((event: MessageEvent) => void) | null,
      onerror: null,
      onclose: null,
      send: vi.fn(),
      close: vi.fn(),
    }
    class FakePeer {
      iceGatheringState = 'complete'
      localDescription = { sdp: 'offer' }
      ontrack = null
      onconnectionstatechange = null
      addTrack = vi.fn()
      createDataChannel = () => dataChannel
      createOffer = async () => ({ type: 'offer', sdp: 'offer' })
      setLocalDescription = async () => {}
      setRemoteDescription = async () => {}
      close = vi.fn()
    }
    vi.stubGlobal('RTCPeerConnection', FakePeer)
    const closeAudio = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('AudioContext', class {
      createAnalyser() { throw new Error('analyser unavailable') }
      close = closeAudio
    })
    getUserMedia.mockResolvedValue(stream)
    mocks.settings.mockResolvedValue({ voice: { channel: 'live' } })
    mocks.liveStart.mockResolvedValue({ conversationId: 'conversation', answerSdp: 'answer' })
    mocks.delegate.mockResolvedValue('Done')
    const { result } = callHook()
    await act(async () => { result.current.start() })
    expect(closeAudio).toHaveBeenCalledOnce()
    expect(mocks.liveStart).toHaveBeenCalledWith({ offerSdp: 'offer', providerId: 'provider', model: 'model' })
    const receive = (event: object) => dataChannel.onmessage?.({ data: JSON.stringify(event) } as MessageEvent)
    await act(async () => {
      receive({ type: 'session.started' })
      receive({ type: 'session.input_transcript.delta', delta: 'Hi' })
      receive({ type: 'session.output_transcript.delta', delta: 'OK' })
      receive({ type: 'session.delegation.created', delegation: { id: 'work' } })
    })
    expect(result.current.state).toBe('active')
    expect(result.current.transcriptRows).toEqual([
      { id: 'voice-1', role: 'user', text: 'Hi', finalized: true },
      { id: 'voice-2', role: 'assistant', text: 'OK', finalized: false },
    ])
    expect(mocks.delegate).toHaveBeenCalledWith({ conversationId: 'conversation', request: '用户：Hi\n助手：OK' })
    expect(dataChannel.send).toHaveBeenCalledWith(expect.stringContaining('session.commentary.append'))
    expect(onDelegationComplete).toHaveBeenCalledOnce()
    act(() => result.current.toggleMuted())
    expect(track.enabled).toBe(false)
    expect(result.current.muted).toBe(true)
    act(() => result.current.hangup())
    expect(dataChannel.send).toHaveBeenCalledWith(JSON.stringify({ type: 'session.close' }))
    act(() => receive({ type: 'session.closed' }))
    expect(track.stop).toHaveBeenCalledOnce()
    expect(result.current.state).toBe('idle')
    expect(result.current.transcriptRows).toEqual([])
  })

  it('stops a stream granted after cancellation', async () => {
    const media = deferred<MediaStream>()
    const stop = vi.fn()
    getUserMedia.mockReturnValue(media.promise)
    mocks.settings.mockResolvedValue({ voice: { channel: 'live' } })
    const { result } = callHook()
    await act(async () => { result.current.start() })
    act(() => result.current.hangup())
    expect(result.current.state).toBe('idle')
    expect(mocks.listen).not.toHaveBeenCalled()
    mocks.settings.mockResolvedValue({ voice: { channel: 'auto' } })
    await act(async () => { result.current.start() })
    emit(0, { sessionId: 'session', type: 'active' })
    await act(async () => { media.resolve({ getTracks: () => [{ stop }] } as unknown as MediaStream) })
    expect(stop).toHaveBeenCalledOnce()
    expect(result.current.state).toBe('active')
    expect(result.current.channel).toBe('codex')
  })

  it('times out a pending microphone and stops a stream granted later', async () => {
    vi.useFakeTimers()
    const media = deferred<MediaStream>()
    const stop = vi.fn()
    getUserMedia.mockReturnValue(media.promise)
    mocks.settings.mockResolvedValue({ voice: { channel: 'live' } })
    const { result } = callHook()
    await act(async () => { result.current.start() })
    expect(getUserMedia).toHaveBeenCalledOnce()
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000) })
    expect(result.current.state).toBe('error')
    expect(result.current.lastError).toContain('microphone permission')
    await act(async () => { media.resolve({ getTracks: () => [{ stop }] } as unknown as MediaStream) })
    expect(stop).toHaveBeenCalledOnce()
  })
})
