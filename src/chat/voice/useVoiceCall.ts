import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type MutableRefObject, type RefObject } from 'react'
import { api, type VoiceCodexEvent } from '../../api/tauri'
import { getSettingsCached } from '../../api/settingsCache'
import { notifyError } from '../../components/appNoticeStore'
import { i18n, type Lang } from '../../components/i18n'
import type { OrbState } from './orb/engine'

export type VoiceRole = 'user' | 'assistant'
export type VoiceChannel = 'codex' | 'live'
/** `voice_codex_start` error prefix meaning Codex voice cannot be used here (auto mode falls back to GPT-Live). */
const CODEX_UNAVAILABLE = 'unavailable:'
// shortcut: retain 32 recent turns of at most 4096 characters; increase if longer live history is needed.
const CAPTION_TURNS = 32
const CAPTION_CHARACTERS = 4096
const COMPLETED_ITEMS = 1024
export type VoiceState = 'idle' | 'connecting' | 'active' | 'closing' | 'error'

export type VoiceTranscript = { request: string; role: VoiceRole | null }
export type VoiceTranscriptRow = { id: string; role: VoiceRole | 'unknown'; text: string; finalized: boolean }

export function emptyVoiceTranscript(): VoiceTranscript {
  return { request: '', role: null }
}

export function appendTranscriptDelta(transcript: VoiceTranscript, role: VoiceRole, delta: string): VoiceTranscript {
  if (!delta) return transcript
  const prefix = transcript.role === role ? '' : `${transcript.request ? '\n' : ''}${role === 'user' ? '用户：' : '助手：'}`
  return { request: transcript.request + prefix + delta, role }
}

export function takeVoiceTranscript(transcript: VoiceTranscript): { request: string; transcript: VoiceTranscript } {
  return { request: transcript.request, transcript: emptyVoiceTranscript() }
}

/** Speaker energy can temporarily override this resting state in the canvas renderer. */
export function voiceOrbState(state: VoiceState, muted: boolean, working: number): OrbState {
  if (state === 'connecting' || state === 'closing') return 'connecting'
  if (working > 0) return 'working'
  if (muted) return 'breathing'
  return 'listening'
}

export function formatElapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  const pad = (value: number) => String(value).padStart(2, '0')
  const hours = Math.floor(s / 3600)
  const minutes = Math.floor((s % 3600) / 60)
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(s % 60)}` : `${minutes}:${pad(s % 60)}`
}

export type VoiceLevels = { mic: number; speaker: number }

type Delegation = { id: string; request: string }

type VoiceTurn = VoiceTranscriptRow & { itemId?: string }

type VoiceCall = {
  abort: AbortController
  stream: MediaStream | null
  peer: RTCPeerConnection | null
  channel: RTCDataChannel | null
  audio: HTMLAudioElement | null
  remoteStream: MediaStream | null
  audioContext: AudioContext | null
  micAnalyser: AnalyserNode | null
  speakerAnalyser: AnalyserNode | null
  levelFrame: number | null
  closeTimer: number | null
  started: boolean
  remoteReady: boolean
  closing: boolean
  conversationId: string | null
  transcript: VoiceTranscript
  pending: Delegation[]
  working: number
  turns: VoiceTurn[]
  turnSequence: number
  completedItems: Set<string>
  muting: boolean
  /** Codex subscription call: a reserved native session, including while connecting. */
  codexSession: string | null
  codexUnlisten: (() => void) | null
}

type VoiceEvent = {
  type?: string
  delta?: unknown
  delegation?: { id?: unknown }
  error?: unknown
  message?: unknown
}

function errorDetail(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  if (error && typeof error === 'object' && 'message' in error && typeof error.message === 'string') return error.message
  return ''
}

function waitForIceGathering(peer: RTCPeerConnection, signal: AbortSignal, timeoutMessage: string): Promise<void> {
  if (signal.aborted || peer.iceGatheringState === 'complete') return Promise.resolve()
  return new Promise((resolve, reject) => {
    let settled = false
    const finish = (error?: Error) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      peer.removeEventListener('icegatheringstatechange', onStateChange)
      signal.removeEventListener('abort', onAbort)
      if (error) reject(error)
      else resolve()
    }
    const onStateChange = () => { if (peer.iceGatheringState === 'complete') finish() }
    const onAbort = () => finish()
    const timer = window.setTimeout(() => finish(new Error(timeoutMessage)), 10_000)
    peer.addEventListener('icegatheringstatechange', onStateChange)
    signal.addEventListener('abort', onAbort)
    if (signal.aborted) finish()
    else onStateChange()
  })
}

function requestMicrophone(signal: AbortSignal, timeoutMessage: string): Promise<MediaStream> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'))
  const request = navigator.mediaDevices.getUserMedia({ audio: true })
  return new Promise((resolve, reject) => {
    let settled = false
    const settle = () => {
      if (settled) return false
      settled = true
      clearTimeout(timer)
      signal.removeEventListener('abort', onAbort)
      return true
    }
    const onAbort = () => {
      if (settle()) reject(new DOMException('Aborted', 'AbortError'))
    }
    const timer = window.setTimeout(() => {
      if (settle()) reject(new Error(timeoutMessage))
    }, 30_000)
    signal.addEventListener('abort', onAbort)
    if (signal.aborted) onAbort()
    request.then(stream => {
      if (settle()) resolve(stream)
      else stream.getTracks().forEach(track => track.stop())
    }, error => {
      if (settle()) reject(error)
    })
  })
}

export function commentaryContent(content: string): string {
  const limited = content.slice(0, 1200)
  const safe = /[\uD800-\uDBFF]$/.test(limited) ? limited.slice(0, -1) : limited
  const bytes = new TextEncoder().encode(safe)
  if (bytes.length <= 500 && safe.length === content.length) return safe
  // shortcut: UTF-8 bytes conservatively underuse the 500-token limit; use an exact tokenizer for longer replies.
  return new TextDecoder().decode(bytes.subarray(0, 497), { stream: true }) + '…'
}

function analyserFor(context: AudioContext, stream: MediaStream): AnalyserNode {
  const analyser = context.createAnalyser()
  analyser.fftSize = 512
  context.createMediaStreamSource(stream).connect(analyser)
  return analyser
}

function peakLevel(analyser: AnalyserNode | null, buffer: Float32Array<ArrayBuffer>): number {
  if (!analyser) return 0
  analyser.getFloatTimeDomainData(buffer)
  let peak = 0
  for (const sample of buffer) peak = Math.max(peak, Math.abs(sample))
  return Math.min(1, peak)
}

export type VoiceCallApi = {
  state: VoiceState
  muted: boolean
  muting: boolean
  working: number
  transcriptRows: VoiceTranscriptRow[]
  conversationId: string | null
  /** Which voice channel carries the live call. */
  channel: VoiceChannel | null
  /** Epoch ms of `session.started`; drives the call clock. */
  activeSince: number | null
  /** Last start/call failure, shown on the trigger until the next start. */
  lastError: string | null
  stageOpen: boolean
  setStageOpen: (open: boolean) => void
  /** Live peaks (0–1), written every frame without re-rendering. */
  levelsRef: MutableRefObject<VoiceLevels>
  audioRef: RefObject<HTMLAudioElement>
  start: () => void
  hangup: () => void
  toggleMuted: () => void
}

export const VoiceCallContext = createContext<VoiceCallApi | null>(null)

export function useVoiceCallContext(): VoiceCallApi | null {
  return useContext(VoiceCallContext)
}

export function useVoiceCall({ providerId, model, lang, onDelegationComplete }: {
  providerId: string
  model: string
  lang: Lang
  onDelegationComplete: () => void
}): VoiceCallApi {
  const [state, setState] = useState<VoiceState>('idle')
  const [muted, setMuted] = useState(false)
  const [muting, setMuting] = useState(false)
  const [working, setWorking] = useState(0)
  const [transcriptRows, setTranscriptRows] = useState<VoiceTranscriptRow[]>([])
  const [conversationId, setConversationId] = useState<string | null>(null)
  const [activeSince, setActiveSince] = useState<number | null>(null)
  const [lastError, setLastError] = useState<string | null>(null)
  const [stageOpen, setStageOpen] = useState(false)
  const [channel, setChannel] = useState<VoiceChannel | null>(null)
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const levelsRef = useRef<VoiceLevels>({ mic: 0, speaker: 0 })
  const currentRef = useRef<VoiceCall | null>(null)
  const mountedRef = useRef(true)
  const mutedRef = useRef(false)
  const optionsRef = useRef({ lang, onDelegationComplete })
  optionsRef.current = { lang, onDelegationComplete }

  const reportError = useCallback((error: unknown) => {
    const summary = i18n[optionsRef.current.lang].voiceError
    const detail = errorDetail(error)
    notifyError(detail && detail !== summary ? `${summary}: ${detail}` : summary)
    if (mountedRef.current) setLastError(detail || summary)
  }, [])

  const dispose = useCallback((call: VoiceCall, nextState: VoiceState) => {
    if (currentRef.current !== call) return
    currentRef.current = null
    call.abort.abort()
    call.codexUnlisten?.()
    call.codexUnlisten = null
    clearTimeout(call.closeTimer ?? undefined)
    if (call.levelFrame !== null) cancelAnimationFrame(call.levelFrame)
    void call.audioContext?.close().catch(() => {})
    levelsRef.current = { mic: 0, speaker: 0 }
    if (call.channel) {
      call.channel.onmessage = null
      call.channel.onerror = null
      call.channel.onclose = null
      call.channel.close()
    }
    if (call.peer) {
      call.peer.ontrack = null
      call.peer.onconnectionstatechange = null
      call.peer.close()
    }
    call.stream?.getTracks().forEach(track => track.stop())
    if (call.audio && call.audio.srcObject === call.remoteStream) call.audio.srcObject = null
    mutedRef.current = false
    if (mountedRef.current) {
      setMuted(false)
      setMuting(false)
      setWorking(0)
      setTranscriptRows([])
      setConversationId(null)
      setChannel(null)
      setActiveSince(null)
      setStageOpen(false)
      setState(nextState)
    }
  }, [])

  const fail = useCallback((call: VoiceCall, error: unknown) => {
    if (currentRef.current !== call || call.closing) return
    reportError(error)
    dispose(call, 'error')
  }, [dispose, reportError])

  const flushDelegations = useCallback((call: VoiceCall) => {
    if (currentRef.current !== call || call.closing || !call.started || !call.conversationId) return
    const id = call.conversationId
    const delegations = call.pending.splice(0)
    for (const delegation of delegations) {
      if (currentRef.current !== call || call.closing) break
      call.working += 1
      setWorking(call.working)
      void (async () => {
        let content: string
        try {
          content = await api.voiceDelegate({ conversationId: id, request: delegation.request })
        } catch (error) {
          if (currentRef.current !== call || call.closing) return
          reportError(error)
          content = `出错了：${errorDetail(error) || i18n[optionsRef.current.lang].voiceError}`
        }
        if (currentRef.current !== call || call.closing) return
        let sendFailure: { error: unknown } | null = null
        try {
          if (call.channel?.readyState !== 'open') throw new Error(i18n[optionsRef.current.lang].voiceError)
          call.channel.send(JSON.stringify({
            type: 'session.commentary.append',
            event_id: crypto.randomUUID(),
            delegation_id: delegation.id,
            content: commentaryContent(content),
          }))
        } catch (error) {
          sendFailure = { error }
        }
        if (currentRef.current !== call || call.closing) return
        call.working -= 1
        setWorking(call.working)
        try {
          optionsRef.current.onDelegationComplete()
        } finally {
          if (sendFailure) fail(call, sendFailure.error)
        }
      })()
    }
  }, [fail, reportError])

  const start = useCallback(() => {
    if (currentRef.current) return
    const call: VoiceCall = {
      abort: new AbortController(), stream: null, peer: null, channel: null, audio: null, remoteStream: null,
      audioContext: null, micAnalyser: null, speakerAnalyser: null, levelFrame: null,
      closeTimer: null, started: false, remoteReady: false, closing: false, conversationId: null,
      transcript: emptyVoiceTranscript(), pending: [], working: 0,
      codexSession: null, codexUnlisten: null, turns: [], turnSequence: 0, completedItems: new Set(), muting: false,
    }
    currentRef.current = call
    mutedRef.current = false
    setMuted(false)
    setMuting(false)
    setWorking(0)
    setTranscriptRows([])
    setConversationId(null)
    setActiveSince(null)
    setLastError(null)
    setStageOpen(true)
    setState('connecting')
    const isCurrent = () => currentRef.current === call && !call.closing
    const publishTranscript = () => setTranscriptRows(call.turns.map(({ id, role, text, finalized }) => ({ id, role, text, finalized })))
    const createTurn = (role: VoiceTranscriptRow['role'], itemId?: string, beforeLast = false): VoiceTurn => {
      const turn: VoiceTurn = { id: `voice-${++call.turnSequence}`, itemId, role, text: '', finalized: false }
      if (beforeLast) call.turns.splice(call.turns.length - 1, 0, turn)
      else call.turns.push(turn)
      if (call.turns.length > CAPTION_TURNS) call.turns.shift()
      return turn
    }
    const startLive = async () => {
      setChannel('live')
      try {
        const stream = await requestMicrophone(call.abort.signal, optionsRef.current.lang === 'zh'
          ? '等待麦克风权限超时，请重试。'
          : 'Timed out waiting for microphone permission. Please try again.')
        if (!isCurrent()) {
          stream.getTracks().forEach(track => track.stop())
          return
        }
        call.stream = stream
        stream.getAudioTracks().forEach(track => { track.enabled = !mutedRef.current })
        // Level meters only animate the orb; a missing AudioContext must not fail the call.
        try {
          call.audioContext = new AudioContext()
          call.micAnalyser = analyserFor(call.audioContext, stream)
          const buffer = new Float32Array(512)
          const tick = () => {
            if (currentRef.current !== call) return
            levelsRef.current = {
              mic: mutedRef.current ? 0 : peakLevel(call.micAnalyser, buffer),
              speaker: peakLevel(call.speakerAnalyser, buffer),
            }
            call.levelFrame = requestAnimationFrame(tick)
          }
          call.levelFrame = requestAnimationFrame(tick)
        } catch {
          void call.audioContext?.close().catch(() => {})
          call.audioContext = null
        }
        const peer = new RTCPeerConnection()
        call.peer = peer
        peer.ontrack = event => {
          if (!isCurrent()) return
          if (call.audio && call.audio.srcObject === call.remoteStream) call.audio.srcObject = null
          call.remoteStream = event.streams[0] ?? new MediaStream([event.track])
          call.audio = audioRef.current
          if (call.audio) call.audio.srcObject = call.remoteStream
          if (call.audioContext) {
            try {
              call.speakerAnalyser = analyserFor(call.audioContext, call.remoteStream)
            } catch {
              call.speakerAnalyser = null
            }
          }
        }
        peer.onconnectionstatechange = () => {
          if (peer.connectionState !== 'failed' && peer.connectionState !== 'closed') return
          if (call.closing) dispose(call, 'idle')
          else fail(call, null)
        }
        stream.getTracks().forEach(track => peer.addTrack(track, stream))
        const channel = peer.createDataChannel('oai-events')
        call.channel = channel
        channel.onmessage = message => {
          if (currentRef.current !== call) return
          let event: VoiceEvent
          try {
            if (typeof message.data !== 'string') throw new Error(i18n[optionsRef.current.lang].voiceError)
            const parsed: unknown = JSON.parse(message.data)
            if (!parsed || typeof parsed !== 'object') throw new Error(i18n[optionsRef.current.lang].voiceError)
            event = parsed as VoiceEvent
          } catch (error) {
            fail(call, error)
            return
          }
          if (event.type === 'session.closed') {
            dispose(call, 'idle')
            return
          }
          if (event.type === 'error' || event.type === 'session.error') {
            if (call.closing) {
              reportError(event.error ?? event.message)
              dispose(call, 'idle')
            } else fail(call, event.error ?? event.message)
            return
          }
          if (call.closing) return
          if (event.type === 'session.started') {
            call.started = true
            setActiveSince(Date.now())
            if (call.remoteReady) setState('active')
            flushDelegations(call)
          } else if (event.type === 'session.input_transcript.delta' || event.type === 'session.output_transcript.delta') {
            if (typeof event.delta !== 'string' || !event.delta) return
            const role = event.type === 'session.input_transcript.delta' ? 'user' : 'assistant'
            call.transcript = appendTranscriptDelta(call.transcript, role, event.delta)
            let turn = call.turns[call.turns.length - 1]
            if (!turn || turn.role !== role || turn.finalized) {
              if (turn) turn.finalized = true
              turn = createTurn(role)
            }
            turn.text = Array.from(turn.text + event.delta).slice(-CAPTION_CHARACTERS).join('')
            publishTranscript()
          } else if (event.type === 'session.delegation.created') {
            if (typeof event.delegation?.id !== 'string' || !event.delegation.id) return
            const taken = takeVoiceTranscript(call.transcript)
            call.transcript = taken.transcript
            call.pending.push({ id: event.delegation.id, request: taken.request })
            flushDelegations(call)
          }
        }
        channel.onerror = event => {
          if (currentRef.current !== call) return
          if (call.closing) {
            reportError(event)
            dispose(call, 'idle')
          } else fail(call, event)
        }
        channel.onclose = () => {
          if (currentRef.current !== call) return
          if (call.closing) dispose(call, 'idle')
          else fail(call, null)
        }
        const offer = await peer.createOffer()
        if (!isCurrent()) return
        await peer.setLocalDescription(offer)
        if (!isCurrent()) return
        await waitForIceGathering(peer, call.abort.signal, i18n[optionsRef.current.lang].voiceError)
        if (!isCurrent()) return
        const offerSdp = peer.localDescription?.sdp
        if (!offerSdp) throw new Error(i18n[optionsRef.current.lang].voiceError)
        const answer = await api.voiceLiveStart({ offerSdp, providerId, model })
        if (!isCurrent()) return
        call.conversationId = answer.conversationId
        setConversationId(answer.conversationId)
        flushDelegations(call)
        await peer.setRemoteDescription({ type: 'answer', sdp: answer.answerSdp })
        if (!isCurrent()) return
        call.remoteReady = true
        if (call.started) setState('active')
      } catch (error) {
        if (isCurrent()) fail(call, error)
      }
    }

    const handleCodex = (event: VoiceCodexEvent) => {
      if (currentRef.current !== call || (call.closing && event.type !== 'closed')) return
      switch (event.type) {
        case 'active':
          call.started = true
          setActiveSince(Date.now())
          setState('active')
          break
        case 'partial': {
          if (!event.delta || (event.itemId && call.completedItems.has(event.itemId))) break
          const turns = call.turns
          const last = turns[turns.length - 1]
          let turn = event.itemId ? turns.find(item => item.itemId === event.itemId) : undefined
          if (!turn && last && !last.finalized && (!event.itemId || !last.itemId)
            && (!event.role || last.role === 'unknown' || event.role === last.role)) {
            turn = last
            if (event.itemId) turn.itemId = event.itemId
          }
          if (!turn) turn = createTurn(event.role ?? 'unknown', event.itemId)
          if (turn.finalized) break
          if (event.role) turn.role = event.role
          turn.text = Array.from(turn.text + event.delta).slice(-CAPTION_CHARACTERS).join('')
          publishTranscript()
          break
        }
        case 'final': {
          const turns = call.turns
          const last = turns[turns.length - 1]
          let turn = turns.find(item => item.itemId === event.itemId)
          if (call.completedItems.has(event.itemId)) break
          if (!turn && last && !last.itemId && !last.finalized && (last.role === 'unknown' || last.role === event.role)) {
            turn = last
          }
          if (!turn) {
            // A final without an observed partial precedes the turn already streaming.
            turn = createTurn(event.role, event.itemId, !!last && !last.finalized)
          }
          turn.itemId = event.itemId
          turn.role = event.role
          // Flat done events can finalize an older prefix after newer speech has arrived.
          if (!event.preserveLonger || !turn.text.startsWith(event.text)) {
            turn.text = Array.from(event.text).slice(-CAPTION_CHARACTERS).join('')
          }
          turn.finalized = true
          call.completedItems.add(event.itemId)
          if (call.completedItems.size > COMPLETED_ITEMS) {
            const oldest = call.completedItems.values().next().value
            if (oldest !== undefined) call.completedItems.delete(oldest)
          }
          publishTranscript()
          break
        }
        case 'levels':
          levelsRef.current = { mic: mutedRef.current ? 0 : event.mic, speaker: event.speaker }
          break
        case 'working':
          if (event.count < call.working) optionsRef.current.onDelegationComplete()
          call.working = Math.max(0, event.count)
          setWorking(call.working)
          break
        case 'closed':
          if (event.error && !call.closing) fail(call, event.error)
          else {
            if (event.error) reportError(event.error)
            dispose(call, 'idle')
          }
          break
      }
    }

    /** Resolves false when Codex voice is unavailable and the caller may fall back. */
    const startCodex = async (mayFallBack: boolean): Promise<boolean> => {
      let sessionId: string | null = null
      let started = false
      const buffered: VoiceCodexEvent[] = []
      // Subscribe before starting so `active` and early captions are never missed.
      const unlisten = await api.onVoiceCodexEvent((event) => {
        if (currentRef.current !== call || event.sessionId !== sessionId) return
        if (!started) {
          if (event.type !== 'levels') buffered.push(event)
        } else handleCodex(event)
      })
      if (!isCurrent()) {
        unlisten()
        return true
      }
      call.codexUnlisten = unlisten
      sessionId = await api.voiceCodexReserve()
      if (!isCurrent()) {
        void api.voiceCodexStop(sessionId).catch(() => {})
        return true
      }
      call.codexSession = sessionId
      try {
        await api.voiceCodexStart(sessionId)
      } catch (error) {
        if (!isCurrent()) return true
        try {
          await api.voiceCodexStop(sessionId)
        } catch (cleanupError) {
          if (isCurrent()) throw cleanupError
          return true
        }
        if (!isCurrent()) return true
        call.codexSession = null
        call.codexUnlisten?.()
        call.codexUnlisten = null
        const detail = errorDetail(error)
        if (!detail.startsWith(CODEX_UNAVAILABLE)) throw error
        if (mayFallBack) return false
        throw new Error(detail.slice(CODEX_UNAVAILABLE.length).trim())
      }
      if (!isCurrent()) return true
      started = true
      setChannel('codex')
      for (const event of buffered.splice(0)) if (event.sessionId === sessionId) handleCodex(event)
      return true
    }

    void (async () => {
      try {
        const preferred = (await getSettingsCached()).voice?.channel ?? 'auto'
        if (!isCurrent()) return
        if (preferred !== 'live' && await startCodex(preferred !== 'codex')) return
        if (isCurrent()) await startLive()
      } catch (error) {
        if (isCurrent()) fail(call, error)
      }
    })()
  }, [dispose, fail, flushDelegations, model, providerId, reportError])

  const hangup = useCallback(() => {
    const call = currentRef.current
    if (!call || call.closing) return
    call.closing = true
    if (call.codexSession) {
      const session = call.codexSession
      if (mountedRef.current) {
        setWorking(0)
        setState('closing')
      }
      void api.voiceCodexStop(session).catch(error => {
        if (currentRef.current === call) reportError(error)
      }).finally(() => dispose(call, 'idle'))
      return
    }
    call.abort.abort()
    call.stream?.getAudioTracks().forEach(track => { track.enabled = false })
    if (mountedRef.current) {
      setWorking(0)
      setState('closing')
    }
    if (call.started && call.channel?.readyState === 'open') {
      call.closeTimer = window.setTimeout(() => dispose(call, 'idle'), 15_000)
      try {
        call.channel.send(JSON.stringify({ type: 'session.close' }))
      } catch (error) {
        reportError(error)
        dispose(call, 'idle')
      }
    } else dispose(call, 'idle')
  }, [dispose, reportError])

  const toggleMuted = useCallback(() => {
    const call = currentRef.current
    if (!call || !call.started || call.closing || call.muting) return
    const next = !mutedRef.current
    if (call.codexSession) {
      call.muting = true
      setMuting(true)
      void api.voiceCodexMute(call.codexSession, next).then(() => {
        if (currentRef.current !== call || call.closing) return
        mutedRef.current = next
        setMuted(next)
      }).catch(error => {
        if (currentRef.current === call && !call.closing) reportError(error)
      }).finally(() => {
        if (currentRef.current !== call || call.closing) return
        call.muting = false
        setMuting(false)
      })
      return
    }
    mutedRef.current = next
    call.stream?.getAudioTracks().forEach(track => { track.enabled = !next })
    setMuted(next)
  }, [reportError])

  useEffect(() => {
    const call = currentRef.current
    if (!call?.remoteStream || !audioRef.current || call.audio === audioRef.current) return
    if (call.audio && call.audio.srcObject === call.remoteStream) call.audio.srcObject = null
    call.audio = audioRef.current
    call.audio.srcObject = call.remoteStream
  })

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      const call = currentRef.current
      hangup()
      if (call) dispose(call, 'idle')
    }
  }, [dispose, hangup])

  return useMemo(() => ({
    state, muted, muting, working, transcriptRows, conversationId, channel, activeSince, lastError, stageOpen, setStageOpen,
    levelsRef, audioRef, start, hangup, toggleMuted,
  }), [state, muted, muting, working, transcriptRows, conversationId, channel, activeSince, lastError, stageOpen, start, hangup, toggleMuted])
}
