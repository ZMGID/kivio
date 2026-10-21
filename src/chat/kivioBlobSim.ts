import {
  BODY_R,
  CX,
  CY,
  EYE_POINTS,
  bodyPoints,
  faceBlinks,
  facePoints,
  polyPath,
  type BodyShape,
  type FaceName,
  type Pt,
} from './kivioBlobShapes'
import { createBlobProjector } from './kivioBlobVolume'

export { CX, CY, BODY_R, EYE_POINTS, polyPath }
export type { BodyShape, FaceName }

export const BLOB_BLUE = '#1d6bf0'
export const BLOB_EYE = '#f3efe6'
export const BLOB_POKE_RED = '#e23b2e'
const BLOB_ERROR = '#c45c2a'
// 默认朝屏幕右上方看；互动结束后回到同一静止姿态。
const REST_GAZE = { x: 12, y: -10 } as const

function mixHex(a: string, b: string, t: number): string {
  const u = clamp(t, 0, 1)
  const n = (h: string, i: number) => parseInt(h.slice(1 + i * 2, 3 + i * 2), 16)
  const m = (i: number) => Math.round(n(a, i) + (n(b, i) - n(a, i)) * u)
  return `#${[0, 1, 2].map((i) => m(i).toString(16).padStart(2, '0')).join('')}`
}

/**
 * 跟生成过程对齐的心情。idle 闲着；think / search / work / speak 是一轮生成里的四个阶段；
 * error 翻车；done 是刚收工那两秒的小得意；wait 是停下来等用户（问问题 / 等审批）。
 */
export type BlobMood = 'idle' | 'think' | 'search' | 'work' | 'speak' | 'error' | 'done' | 'wait'

// 显式标注:两表三元取用后是同一类型,元组才能直接 spread 进 stepSpring(TS2556)。
type SpringTable = Record<
  'spin' | 'x' | 'y' | 'squash' | 'blink' | 'gaze' | 'morph' | 'body' | 'boost',
  readonly [number, number]
>
const SPR: SpringTable = {
  spin: [5, 0.9],
  x: [3.5, 1],
  y: [4, 1],
  squash: [10, 0.8],
  blink: [26, 1],
  gaze: [13, 0.82],
  morph: [7, 1],
  body: [6, 0.92],
  boost: [9, 0.85],
}

/** Idle turns remain responsive, then settle completely instead of looping. */
const SPR_IDLE: SpringTable = {
  spin: [9, 0.86],
  x: [7, 1],
  y: [8, 0.9],
  squash: [15, 0.72],
  blink: [24, 0.9],
  gaze: [11, 0.82],
  morph: [12, 0.9],
  body: [13, 0.88],
  boost: [10, 0.9],
}

/** 每个心情轮播的脸。第 0 张是进入该心情时先摆的那张。 */
const FACE_PLAY: Record<BlobMood, FaceName[]> = {
  idle: ['neutral'],
  think: ['lookUp', 'hmm', 'lines', 'neutral', 'lookUp', 'focus'],
  search: ['wide', 'peek', 'dots', 'wide', 'neutral'],
  work: ['focus', 'lines', 'focus', 'neutral', 'dots'],
  speak: ['neutral', 'dots', 'happy', 'neutral', 'hmm'],
  error: ['dizzy', 'worry', 'tiny', 'dizzy', 'lines'],
  done: ['happy', 'sparkle', 'happy', 'content'],
  wait: ['wide', 'neutral', 'wide', 'hmm'],
}
const FACE_HOLD: Record<BlobMood, [number, number]> = {
  idle: [5000, 18000],
  think: [2000, 3600],
  search: [1000, 1800],
  work: [1800, 3200],
  speak: [2800, 5000],
  error: [2200, 3800],
  done: [700, 1100],
  wait: [2600, 4800],
}

/**
 * 身体形态轮播：状态借一个形态来说话（思考=云、说话=气泡……）。但不是每轮都借——进入心情时
 * 先按 BODY_CHANCE 掷一次，没中这一趟就老老实实当圆；每次都变云、变气泡就成了固定图标，不像小动作。
 */
const BODY_PLAY: Record<BlobMood, BodyShape[]> = {
  idle: ['circle'],
  think: ['cloud', 'cloud', 'circle', 'cloud'],
  search: ['circle'],
  work: ['squircle', 'squircle', 'circle', 'squircle'],
  speak: ['bubble', 'circle', 'bubble', 'bubble'],
  error: ['puddle'],
  done: ['circle'],
  wait: ['circle'],
}
const BODY_CHANCE: Record<BlobMood, number> = {
  idle: 1,
  think: 0.35,
  search: 0.4,
  work: 0.4,
  speak: 0.25,
  error: 1,
  done: 1,
  wait: 0.5,
}
const BODY_HOLD: Record<BlobMood, [number, number]> = {
  idle: [1e9, 1e9],
  think: [3500, 7000],
  search: [2500, 5000],
  work: [3000, 6500],
  speak: [3000, 6000],
  error: [1e9, 1e9],
  done: [1e9, 1e9],
  wait: [4000, 8000],
}
const BLINK: Record<BlobMood, [number, number] | null> = {
  idle: null,
  think: [3500, 7000],
  search: [1600, 4000],
  work: [2800, 5500],
  speak: [3000, 7000],
  error: [3500, 7000],
  done: null,
  wait: [2200, 4800],
}
const WINK = new Set<BlobMood>(['speak', 'done'])

type Spring = { x: number; v: number; t: number }

const clamp = (n: number, a: number, b: number) => Math.min(b, Math.max(a, n))

function spring(x: number): Spring {
  return { x, v: 0, t: x }
}

/** Closed-form damped oscillator. Retargeting keeps position AND velocity.
 * Frequencies are radians/second; this remains stable across frame rates.
 * See docs/design/momo-volume-motion.md for the derivation/reference.
 */
function stepSpring(s: Spring, freq: number, damp: number, dt: number) {
  if (dt <= 0) return
  const x = s.x - s.t, v = s.v
  const decay = Math.exp(-damp * freq * dt)
  if (damp < 1) {
    const w = freq * Math.sqrt(1 - damp * damp)
    const c = Math.cos(w * dt), sn = Math.sin(w * dt)
    const b = (v + damp * freq * x) / w
    s.x = s.t + decay * (x * c + b * sn)
    s.v = decay * (v * c - (damp * freq * b + w * x) * sn)
  } else {
    const b = v + freq * x
    s.x = s.t + decay * (x + b * dt)
    s.v = decay * (v - freq * b * dt)
  }
  if (!Number.isFinite(s.x) || !Number.isFinite(s.v)) snapSpring(s)
}

const SPRING_AT = 0.05
const SPRING_V = 0.08
const SPRING_UNIT_AT = 0.001
const SPRING_UNIT_V = 0.02

function springAtTarget(s: Spring, at = SPRING_AT, vel = SPRING_V): boolean {
  return Math.abs(s.x - s.t) <= at && Math.abs(s.v) <= vel
}

function snapSpring(s: Spring) {
  s.x = s.t
  s.v = 0
}

/** Each material vertex carries its own momentum across expression/shape
 * changes. Switching targets never restarts a normalized morph timeline.
 */
class PolyMorph {
  key: string
  private points: Pt[][]
  private targets: Pt[][]
  private velocities: Pt[][]
  private resting = true
  constructor(key: string, points: Pt[][]) {
    this.key = key
    this.points = points.map(poly => poly.map(p => [...p]))
    this.targets = points
    this.velocities = points.map(poly => poly.map(() => [0, 0]))
  }
  retarget(key: string, points: Pt[][]) {
    if (key === this.key) return
    this.key = key
    this.targets = points
    this.resting = false
  }
  snap(key: string, points: Pt[][]) {
    this.resting = true
    this.key = key
    this.targets = points
    this.points = points.map(poly => poly.map(p => [...p]))
    this.velocities = points.map(poly => poly.map(() => [0, 0]))
  }
  step(freq: number, damp: number, dt: number) {
    if (this.resting) return
    const s = spring(0)
    this.points.forEach((poly, i) => poly.forEach((p, j) => {
      for (let axis = 0; axis < 2; axis++) {
        s.x = p[axis]; s.v = this.velocities[i][j][axis]; s.t = this.targets[i][j][axis]
        stepSpring(s, freq, damp, dt)
        p[axis] = s.x; this.velocities[i][j][axis] = s.v
      }
    }))
    if (this.settled()) this.finish()
  }
  current(): Pt[][] { return this.points }
  settled(): boolean {
    return this.points.every((poly, i) => poly.every((p, j) => p.every((v, axis) =>
      Math.abs(v - this.targets[i][j][axis]) < SPRING_AT && Math.abs(this.velocities[i][j][axis]) < SPRING_V)))
  }
  finish() { this.snap(this.key, this.targets) }
}

type BlinkKey = { at: number; v: number }

export function queueBlink(q: BlinkKey[], now: number, random: () => number, stretch = 1) {
  const s = stretch
  q.push(
    { at: now, v: 0.05 },
    { at: now + 70 * s, v: 0.05 },
    { at: now + 150 * s, v: 1.08 },
    { at: now + 300 * s, v: 1 },
  )
  if (random() < 0.14) q.push({ at: now + 370 * s, v: 0.05 }, { at: now + 480 * s, v: 1 })
}

export function consumeBlink(q: BlinkKey[], now: number): number | null {
  let key: number | null = null
  while (q.length && now >= q[0].at) key = q.shift()!.v
  return key
}

/** 偏长间隔，避免每次切换都像节拍器。 */
function spanMs(min: number, max: number, u: number): number {
  const t = 1 - (1 - u) * (1 - u)
  return min + (max - min) * t
}

export function isSearchToolName(name: string): boolean {
  const n = name.toLowerCase()
  return n.includes('search') || n.includes('webfetch') || n.includes('web_fetch') || n.includes('knowledge')
}

export function resolveBlobMood(input: {
  active: boolean
  error?: boolean
  contentLen?: number
  reasoningStreaming?: boolean
  runningToolNames?: string[]
  /** 正在等用户回答 / 审批：停下来看着你。 */
  waiting?: boolean
  /** 刚收工（StreamStatusLine 记的短窗口）：小得意一下。 */
  done?: boolean
}): BlobMood {
  if (input.error && !input.active) return 'error'
  if (!input.active) return input.done ? 'done' : 'idle'
  if (input.error) return 'error'
  if (input.waiting) return 'wait'
  const running = input.runningToolNames ?? []
  if (running.some(isSearchToolName)) return 'search'
  if (running.length > 0) return 'work'
  if ((input.contentLen ?? 0) > 0 && !input.reasoningStreaming) return 'speak'
  return 'think'
}

interface Pose {
  spin: number
  tx: number
  ty: number
  squash: number
  lid: number
  boost: number
}

interface IdleBias {
  spin: number
  tx: number
  ty: number
  squash: number
}

interface PoseCtx {
  nodUntil: number
  nodEnd: number
  impulseAt: number
  shakeUntil: number
  hopUntil: number
  biasUntil: number
  bias: IdleBias
}

function applyPose(mood: BlobMood, mt: number, now: number, ctx: PoseCtx, rand: (a: number, b: number) => number): Pose {
  let spin = 0
  let tx = 0
  let ty = 0
  let squash = 1
  let lid = 1
  let boost = 1
  if (mood === 'idle') {
    spin = ctx.bias.spin
    tx = ctx.bias.tx
    ty = ctx.bias.ty
    squash = ctx.bias.squash
    if (now < ctx.shakeUntil) {
      spin += Math.sin(now * 0.055) * 8
      tx += Math.sin(now * 0.08) * 5
    }
  } else if (mood === 'think') {
    // 云是横着飘的，别歪太多；左右慢慢晃 + 轻微上浮。
    spin = -3
    ty = -1.5
  } else if (mood === 'search') {
    spin = 0
  } else if (mood === 'work') {
    const et = Math.sin(mt * Math.PI * 2 * 1.6)
    spin = 5 + et * 3.5
    tx = 4
    ty = 2 + Math.max(0, et) * 4
    squash = 1 - Math.max(0, et) * 0.03
  } else if (mood === 'speak') {
    spin = 3
    tx = 3
    ty = -2.5 + Math.sin(mt * 0.8) * 1.1
    squash = 1.018
    boost = 1.05
    if (now >= ctx.nodUntil) {
      ctx.nodUntil = now + rand(1800, 3200)
      ctx.nodEnd = now + 380
    }
    if (now < ctx.nodEnd) {
      const et = 1 - (ctx.nodEnd - now) / 380
      ty += Math.sin(et * Math.PI) * 6
      spin += Math.sin(et * Math.PI) * 3
    }
  } else if (mood === 'done') {
    // 果冻式小蹦：左右晃 + 上下弹 + 拉伸压扁。
    const et = Math.sin(mt * 2.4)
    spin = Math.sin(mt * 1.2) * 4
    tx = Math.sin(mt * 1.1) * 2.5
    ty = -Math.abs(et) * 3.5
    squash = 1 + et * 0.025
    boost = 1.06
  } else if (mood === 'wait') {
    // 歪头看着你，偶尔慢慢摆一下。
    spin = -4
    tx = 3
    ty = -1 + Math.sin(mt * 0.7) * 0.8
    squash = 1.01
    boost = 1.03
  } else {
    if (now >= ctx.impulseAt) {
      ctx.shakeUntil = now + 420
      ctx.impulseAt = now + rand(1800, 3200)
    }
    spin = now < ctx.shakeUntil ? Math.sin(now * 0.05) * 6 : 0
    ty = 4
    squash = 0.972
    lid = 0.92
  }
  return { spin, tx, ty, squash, lid, boost }
}

function nextGaze(mood: BlobMood, rand: (a: number, b: number) => number, sign: () => number) {
  if (mood === 'think') return { x: sign() * rand(0.5, 1) * 16, y: -rand(0.4, 1) * 10, hold: [1500, 2800] as [number, number] }
  if (mood === 'search') return { x: sign() * rand(0.7, 1) * 16, y: rand(-1, 1) * 10, hold: [550, 1150] as [number, number] }
  if (mood === 'work') return { x: rand(-0.4, 0.4) * 16, y: rand(0.4, 1) * 10, hold: [1200, 2400] as [number, number] }
  if (mood === 'speak') return { x: rand(-0.3, 0.3) * 16, y: rand(-0.25, 0.25) * 10, hold: [2200, 4200] as [number, number] }
  if (mood === 'done') return { x: rand(-0.3, 0.3) * 16, y: -rand(0.1, 0.5) * 10, hold: [900, 1600] as [number, number] }
  if (mood === 'wait') return { x: rand(-0.15, 0.15) * 16, y: -rand(0.5, 0.9) * 10, hold: [1800, 3600] as [number, number] }
  return { x: rand(-0.2, 0.2) * 16, y: 0.2 * 10, hold: [1800, 3200] as [number, number] }
}

function hopCadence(mood: BlobMood): [number, number] | null {
  if (mood === 'search') return [4000, 7000]
  if (mood === 'work') return [6000, 9000]
  if (mood === 'think') return [8000, 12000]
  return null
}

export interface BlobPaint {
  rig: string
  body: string
  bodyD: string
  fill: string
  eyes: [{ d: string; transform: string }, { d: string; transform: string }]
}

/** 有限醒来的夹紧范围。闲置静稳后不再用节拍续帧。 */
export const BLOB_IDLE_WAKE_MIN_MS = 48
export const BLOB_IDLE_WAKE_MAX_MS = 16_000

/** `null` = 不排下一帧（隐藏 / 屏外 / 失焦 / 被覆盖 / 减弱动态 / 已经静稳）。`0` = rAF。`>0` = 睡到下一次有限醒来。 */
export function blobScheduleMs(opts: {
  reducedMotion: boolean
  hidden: boolean
  onScreen: boolean
  unfocused?: boolean
  covered?: boolean
  highFps: boolean
  idleWakeMs?: number
}): number | null {
  if (opts.reducedMotion || opts.hidden || !opts.onScreen || opts.unfocused || opts.covered) return null
  if (opts.highFps) return 0
  const wake = opts.idleWakeMs
  if (wake == null || !Number.isFinite(wake)) return null
  return Math.min(BLOB_IDLE_WAKE_MAX_MS, Math.max(BLOB_IDLE_WAKE_MIN_MS, wake))
}

export interface BlobSimDebug {
  mood: BlobMood
  face: FaceName
  body: BodyShape
  spin: number
  blink: number
  heat: number
  pokes: number
}

export class KivioBlobSim {
  mood: BlobMood = 'idle'
  private random: () => number
  private reduced: boolean
  private spin = spring(0)
  private tx = spring(0)
  private ty = spring(0)
  private squash = spring(1)
  private blink = spring(1)
  private winks = [spring(1), spring(1)]
  private gazeX = spring(REST_GAZE.x)
  private gazeY = spring(REST_GAZE.y)
  private boost = spring(1)
  private face = new PolyMorph('neutral', facePoints('neutral'))
  private body = new PolyMorph('circle', [bodyPoints('circle')])
  private faceIdx = 0
  private bodyIdx = 0
  /** 这一趟心情用的身体轮播：掷中才是 BODY_PLAY，否则只有圆。 */
  private bodyList: BodyShape[] = ['circle']
  private t0 = 0
  private last = 0
  private faceUntil = 0
  private bodyUntil = 0
  private blinkUntil = 0
  private gazeUntil = 0
  private winkAt = -1e9
  private winkEye = 0
  private winkUntil = 0
  private winkDur = 320
  private bounce = spring(0)
  private project = createBlobProjector()
  private pointer: { x: number; y: number } | null = null
  private ambient = false
  private ambientAt = Infinity
  private ambientKind = -1
  private ambientActive = false
  private pokeHeat = 0
  private pokeCount = 0
  private lastPokeAt = -1e9
  private heatHoldUntil = 0
  /** 被戳时身体的临时形态（压扁 / 装方 / 炸毛）。 */
  private pokeShape: BodyShape | null = null
  private pokeShapeUntil = 0
  private blinkQ: BlinkKey[] = []
  private inited = false
  private ctx: PoseCtx = {
    nodUntil: 0,
    nodEnd: 0,
    impulseAt: 0,
    shakeUntil: 0,
    hopUntil: Infinity,
    biasUntil: Infinity,
    bias: { spin: 0, tx: 0, ty: 0, squash: 1 },
  }

  constructor(opts: { random?: () => number; reducedMotion?: boolean } = {}) {
    this.random = opts.random ?? Math.random
    this.reduced = opts.reducedMotion ?? false
  }

  setMood(mood: BlobMood, now = 0) {
    if (mood !== this.mood) this.releaseLook()
    const changed = mood !== this.mood || !this.inited
    this.mood = mood
    const faces = FACE_PLAY[mood]
    this.faceIdx = 0
    this.bodyIdx = 0
    if (changed) {
      this.bodyList = this.random() < BODY_CHANCE[mood] ? BODY_PLAY[mood] : ['circle']
      this.face.retarget(faces[0], facePoints(faces[0]))
      this.body.retarget(this.bodyList[0], [bodyPoints(this.bodyList[0])])
    }
    this.ambientActive = false
    this.ambientAt = now + this.rand(2200, 4200)
    const idle = mood === 'idle'
    this.faceUntil = idle ? Infinity : now + this.rand(...FACE_HOLD[mood])
    this.bodyUntil = idle ? Infinity : now + this.rand(...BODY_HOLD[mood])
    this.blinkUntil = idle ? Infinity : now + this.rand(900, 2800)
    this.gazeUntil = idle ? Infinity : now + this.rand(280, 900)
    if (idle) {
      this.gazeX.t = REST_GAZE.x
      this.gazeY.t = REST_GAZE.y
      this.winkUntil = Infinity
    } else if (WINK.has(mood) && (!Number.isFinite(this.winkUntil) || this.winkUntil <= now)) {
      this.winkUntil = now + this.rand(4000, 8000)
    }
    const hopEvery = hopCadence(mood)
    this.ctx = {
      nodUntil: now + 1600,
      nodEnd: 0,
      impulseAt: now + 600,
      shakeUntil: 0,
      hopUntil: hopEvery ? now + this.rand(...hopEvery) : Infinity,
      biasUntil: Infinity,
      bias: { spin: 0, tx: 0, ty: 0, squash: 1 },
    }
    this.blinkQ = []
    if (mood === 'done' && changed) {
      // 收工先蹦一下。
      this.kick(-95)
    } else if (!idle && faceBlinks(faces[0])) {
      queueBlink(this.blinkQ, now, this.random)
    }
  }

  poke(now: number, lookX?: number, lookY?: number): number {
    this.releaseLook()
    this.ambientActive = false
    this.ambientAt = now + this.rand(12000, 18000)
    this.wake(now)
    if (now - this.lastPokeAt > 4200) this.pokeCount = 0
    this.pokeCount += 1
    this.lastPokeAt = now
    this.pokeHeat = Math.min(1, this.pokeHeat + (this.pokeCount < 4 ? 0.16 : 0.22))
    this.heatHoldUntil = now + 3200
    queueBlink(this.blinkQ, now, this.random, this.mood === 'idle' ? 2.6 : 1)
    const dir = lookX != null ? (lookX < 0 ? -1 : 1) : this.sign()
    const glance = lookX != null ? Math.min(1, Math.max(0.35, Math.abs(lookX))) : this.rand(0.45, 1)
    this.gazeX.t = dir * this.rand(8, 16) * glance
    this.gazeY.t = lookY != null ? clamp(lookY, -1, 1) * 18 : this.rand(-6, 4)
    this.gazeUntil = now + this.rand(this.mood === 'idle' ? 1400 : 700, this.mood === 'idle' ? 2800 : 1400)
    this.kick(-95)
    this.winkAt = now
    this.winkEye = this.random() < 0.5 ? 0 : 1
    this.winkDur = this.mood === 'idle' ? 700 : 320
    // 戳一下压扁；戳多了装方；再戳炸毛。
    if (this.pokeCount >= 7) {
      this.pokeShape = 'burst'
      this.pokeShapeUntil = now + 1400
    } else if (this.pokeCount >= 4) {
      this.pokeShape = 'squircle'
      this.pokeShapeUntil = now + 1800
    } else {
      this.pokeShape = 'puddle'
      this.pokeShapeUntil = now + (this.mood === 'idle' ? 520 : 300)
    }
    if (this.pokeCount >= 7) this.face.retarget('dizzy', facePoints('dizzy'))
    else if (this.pokeCount >= 4) this.face.retarget('lines', facePoints('lines'))
    else if (this.pokeCount >= 2) this.face.retarget('smirk', facePoints('smirk'))
    else this.face.retarget('flat', facePoints('flat'))
    this.faceUntil = now + this.rand(1400, 2600)
    if (this.pokeCount >= 5) this.ctx.shakeUntil = now + 420
    if (this.mood === 'idle') {
      this.ctx.bias = {
        spin: dir * this.rand(3, 7) * (this.pokeCount >= 4 ? 1.6 : 1),
        tx: dir * this.rand(4, 9),
        ty: -3,
        squash: 1.02,
      }
      this.ctx.biasUntil = now + this.span(2500, 8000)
    }
    return this.pokeCount
  }

  lookAt(x: number, y: number, now: number) {
    if (this.reduced || this.mood !== 'idle') return
    this.wake(now)
    if (this.ambientActive) {
      this.ctx.bias = { spin: 0, tx: 0, ty: 0, squash: 1 }
      this.ctx.biasUntil = Infinity
      this.face.retarget('neutral', facePoints('neutral'))
      this.faceUntil = Infinity
      this.ambientActive = false
    }
    this.ambientAt = Infinity
    this.pointer = { x: clamp(x, -1, 1), y: clamp(y, -1, 1) }
    this.gazeX.t = this.pointer.x * 22
    this.gazeY.t = this.pointer.y * 18
    this.gazeUntil = Infinity
  }

  releaseLook(): boolean {
    if (!this.pointer) return false
    this.pointer = null
    this.ambientAt = this.last + this.rand(6000, 12000)
    this.gazeX.t = REST_GAZE.x
    this.gazeY.t = REST_GAZE.y
    this.gazeUntil = Infinity
    return true
  }

  /** Only the large chat character has autonomous idle gestures. */
  setAmbientMotion(enabled: boolean, now: number) {
    this.ambient = enabled
    this.ambientAt = enabled ? now + this.rand(2200, 4200) : Infinity
  }

  private idleGesture(now: number) {
    if (!this.ambient || this.reduced || this.mood !== 'idle' || this.pointer) return
    if (!Number.isFinite(this.ambientAt)) this.ambientAt = now + this.rand(6000, 12000)
    if (now < this.ambientAt) return
    // One authored gesture at a time. Never catch up a queue after hiding.
    this.ambientAt = now + this.rand(6000, 12000)
    if (now - this.last > 20000 || this.wantsHighFps(now) || this.pokeHeat > 0) return
    let kind = Math.floor(this.random() * 5)
    if (kind === this.ambientKind) kind = (kind + 1) % 5
    this.ambientKind = kind
    this.ambientActive = true
    const dir = this.sign(), hold = this.rand(1300, 2300)
    if (kind === 0) {
      // Quiet blink: no whole-body bob on every gesture.
      queueBlink(this.blinkQ, now, this.random, 1.25)
    } else {
      this.gazeX.t = dir * (kind === 1 ? 15 : kind === 2 ? 6 : 9)
      this.gazeY.t = kind === 2 ? -12 : kind === 3 ? 7 : -3
      this.gazeUntil = now + hold
      this.ctx.bias = { spin: dir * (kind === 4 ? 5 : 2), tx: dir * 1.2,
        ty: kind === 2 ? -1.5 : 0, squash: kind === 4 ? 1.012 : 1 }
      this.ctx.biasUntil = now + hold
      const face: FaceName = kind === 2 ? 'hmm' : kind === 4 ? 'content' : 'neutral'
      this.face.retarget(face, facePoints(face))
      this.faceUntil = now + hold
      if (kind === 4) queueBlink(this.blinkQ, now, this.random, 1.4)
    }
  }

  setReducedMotion(reduced: boolean) {
    this.reduced = reduced
    if (reduced) {
      this.releaseLook()
      this.blinkQ = []
      this.winkAt = -1e9
      this.pokeShape = null
      this.ctx.shakeUntil = 0
    }
  }

  private wake(now: number) {
    // A stopped clock must not integrate a new impulse over old idle time.
    if (now - this.last > 80) this.last = now
  }

  private kick(velocity: number) {
    if (this.reduced) return
    this.bounce.v = clamp(this.bounce.v + velocity, -160, 80)
    this.squash.v -= 0.7
  }

  nudge(now: number) {
    this.ambientActive = false
    this.ambientAt = now + this.rand(10000, 15000)
    this.wake(now)
    queueBlink(this.blinkQ, now, this.random, this.mood === 'idle' ? 2.6 : 1)
    this.gazeX.t = this.sign() * this.rand(6, 12)
    this.gazeY.t = this.rand(-4, 3)
    this.gazeUntil = now + this.rand(700, 1400)
  }

  debug(): BlobSimDebug {
    return {
      mood: this.mood,
      face: this.face.key as FaceName,
      body: this.body.key as BodyShape,
      spin: this.spin.x,
      blink: this.blink.x,
      heat: this.pokeHeat,
      pokes: this.pokeCount,
    }
  }

  /** 眨眼、跳跃、形变还没结束时跟帧。闲置只在回稳或热度消退时跟帧。 */
  wantsHighFps(now: number): boolean {
    if (this.reduced) return false
    if (this.blinkQ.length > 0 || this.winks.some(wink => !springAtTarget(wink, SPRING_UNIT_AT, SPRING_UNIT_V))) return true
    if (now < this.winkAt + this.winkDur) return true
    if (!springAtTarget(this.bounce)) return true
    if (now < this.ctx.shakeUntil) return true
    if (this.pokeShape && now < this.pokeShapeUntil + 600) return true
    if (this.mood === 'idle') return (this.pokeHeat > 0 && now >= this.heatHoldUntil) || !this.springsAtTarget()
    if (!this.face.settled() || !this.body.settled()) return true
    if (now < this.ctx.nodEnd) return true
    if (this.mood === 'error') return now < this.ctx.shakeUntil || !this.springsAtTarget()
    return true
  }

  /** 距离下一次必须醒来的毫秒。闲置没有待办时为 Infinity，调用方应停表。 */
  nextIdleWakeMs(now: number): number {
    if (this.mood === 'idle') {
      // Expired holds still need one frame after a hidden/paused interval.
      return Math.min(this.ambient && !this.reduced && !this.pointer ? this.ambientAt : Infinity,
        this.ctx.biasUntil, this.gazeUntil, this.faceUntil,
        this.pokeHeat > 0 ? this.heatHoldUntil : Infinity) - now
    }
    const wakes = [this.blinkUntil, this.faceUntil, this.bodyUntil]
    if (WINK.has(this.mood)) wakes.push(this.winkUntil)
    if (this.mood === 'error') wakes.push(this.ctx.impulseAt)
    return Math.min(...wakes) - now
  }

  sample(now: number): BlobPaint {
    if (!this.inited) {
      this.t0 = now
      this.last = now
      this.setMood(this.mood, now)
      this.inited = true
    }
    this.releaseIdleHolds(now)
    this.idleGesture(now)
    const dt = Math.min((now - this.last) / 1000, 0.08)
    this.last = now
    const mt = (now - this.t0) / 1000
    const pose = applyPose(this.mood, mt, now, this.ctx, (a, b) => this.rand(a, b))
    this.spin.t = pose.spin - this.gazeX.v * 0.10
    this.tx.t = pose.tx
    this.ty.t = pose.ty
    this.squash.t = pose.squash - Math.min(0.07, Math.abs(this.gazeX.v) * 0.0012) + clamp(-this.bounce.v * 0.0007, -0.05, 0.07)
    this.boost.t = pose.boost
    if (this.mood !== 'idle' && now >= this.faceUntil) {
      const list = FACE_PLAY[this.mood]
      this.faceIdx = (this.faceIdx + 1) % list.length
      this.face.retarget(list[this.faceIdx], facePoints(list[this.faceIdx]))
      this.faceUntil = now + this.rand(...FACE_HOLD[this.mood])
    }
    if (this.mood !== 'idle' && now >= this.bodyUntil) {
      this.bodyIdx = (this.bodyIdx + 1) % this.bodyList.length
      this.bodyUntil = now + this.rand(...BODY_HOLD[this.mood])
    }
    if (this.pokeShape && now >= this.pokeShapeUntil) this.pokeShape = null
    const bodyShape = this.bodyTarget(now)
    this.body.retarget(bodyShape, [bodyPoints(bodyShape)])

    // 闭着的脸（笑弯 / x_x / 星星 / 一条线）不眨；但节拍照走，否则 nextIdleWakeMs 会拿到过期时间。
    const blinkable = faceBlinks(this.face.key as FaceName)
    const cad = BLINK[this.mood]
    if (cad && now >= this.blinkUntil) {
      if (blinkable) queueBlink(this.blinkQ, now, this.random)
      this.blinkUntil = now + this.rand(...cad)
    }
    const key = consumeBlink(this.blinkQ, now)
    this.blink.t = blinkable ? key ?? (this.blinkQ.length ? this.blink.t : pose.lid) : 1

    if (this.mood !== 'idle' && now >= this.gazeUntil) {
      const gz = nextGaze(this.mood, (a, b) => this.rand(a, b), () => this.sign())
      this.gazeX.t = gz.x
      this.gazeY.t = gz.y
      this.gazeUntil = now + this.rand(...gz.hold)
    }
    if (WINK.has(this.mood) && now >= this.winkUntil) {
      if (blinkable) {
        this.winkAt = now
        this.winkEye = this.random() < 0.5 ? 0 : 1
        this.winkDur = 320
      }
      this.winkUntil = now + this.rand(4500, 10000)
    }
    const hopEvery = hopCadence(this.mood)
    if (hopEvery && now >= this.ctx.hopUntil && springAtTarget(this.bounce)) {
      this.kick(-95)
      this.ctx.hopUntil = now + this.rand(...hopEvery)
    }

    this.winks.forEach((wink, i) => {
      wink.t = 1
      if (blinkable && i === this.winkEye && now < this.winkAt + this.winkDur) {
        const phase = (now - this.winkAt) / this.winkDur
        wink.t = Math.max(0.04, phase < 0.42 ? 1 - phase / 0.42 : (phase - 0.42) / 0.58)
      }
    })
    const step = Math.max(0, dt)
    const spr = this.mood === 'idle' ? SPR_IDLE : SPR
    {
      stepSpring(this.spin, ...spr.spin, step)
      stepSpring(this.tx, ...spr.x, step)
      stepSpring(this.ty, ...spr.y, step)
      stepSpring(this.squash, ...spr.squash, step)
      stepSpring(this.blink, ...spr.blink, step)
      this.winks.forEach(wink => stepSpring(wink, 25, 0.9, step))
      stepSpring(this.gazeX, ...spr.gaze, step)
      stepSpring(this.gazeY, ...spr.gaze, step)
      this.face.step(...spr.morph, step)
      this.body.step(...(this.pokeShape ? SPR.body : spr.body), step)
      stepSpring(this.bounce, 11, 0.6, step)
      stepSpring(this.boost, ...spr.boost, step)
    }
    if (this.reduced) {
      this.gazeX.t = REST_GAZE.x
      this.gazeY.t = REST_GAZE.y
      snapSpring(this.gazeX)
      snapSpring(this.gazeY)
      for (const s of [this.spin, this.tx, this.ty]) {
        s.t = 0
        snapSpring(s)
      }
      for (const s of [this.squash, this.blink, this.boost, ...this.winks]) {
        s.t = 1
        snapSpring(s)
      }
      snapSpring(this.bounce)
      const faces = FACE_PLAY[this.mood]
      this.face.snap(faces[0], facePoints(faces[0]))
      this.body.snap('circle', [bodyPoints('circle')])
    }

    if (now > this.heatHoldUntil && this.pokeHeat > 0) {
      this.pokeHeat = Math.max(0, this.pokeHeat - dt * 0.32)
      if (this.pokeHeat <= 0.01) {
        this.pokeHeat = 0
        this.pokeCount = 0
      }
    }

    this.finishIdlePose(now)
    const polys = this.face.current()
    const eyes = polys.map((poly, i) => {
      const lid = Math.min(this.blink.x, this.winks[i].x)
      // Blink and expression live on the material, before surface projection.
      const centerY = poly.reduce((sum, p) => sum + p[1], 0) / poly.length
      return poly.map(([x, y]): Pt => [x, centerY + (y - centerY) * clamp(lid, 0.04, 1.2)])
    })
    const volume = this.project(this.body.current()[0], eyes, {
      yaw: clamp(this.gazeX.x * 0.045, -1.15, 1.15),
      pitch: clamp(-this.gazeY.x * 0.04, -0.85, 0.85),
      squash: this.squash.x,
    })
    return {
      rig: `translate(${this.tx.x.toFixed(2)} ${(this.ty.x + this.bounce.x).toFixed(2)}) rotate(${this.spin.x.toFixed(2)} ${CX} ${CY}) translate(${CX} ${CY}) scale(${this.boost.x.toFixed(3)}) translate(${-CX} ${-CY})`,
      // Volume already includes deformation; the face and body never separate.
      body: '',
      bodyD: volume.bodyD,
      fill: this.mood === 'error'
        ? BLOB_ERROR
        : this.pokeHeat <= 0
          ? BLOB_BLUE
          : mixHex(BLOB_BLUE, BLOB_POKE_RED, this.pokeHeat),
      eyes: volume.eyes.map(d => ({ d, transform: '' })) as BlobPaint['eyes'],
    }
  }

  /** 此刻身体该是什么形：被戳 > 出错抖动炸毛 > 心情轮播。闲置停在圆上。 */
  private bodyTarget(now: number): BodyShape {
    if (this.pokeShape) return this.pokeShape
    if (this.mood === 'error') return now < this.ctx.shakeUntil ? 'burst' : 'puddle'
    if (this.mood === 'idle') return 'circle'
    return this.bodyList[this.bodyIdx % this.bodyList.length]
  }

  private springsAtTarget(): boolean {
    return (
      springAtTarget(this.spin) &&
      springAtTarget(this.tx) &&
      springAtTarget(this.ty) &&
      springAtTarget(this.gazeX) &&
      springAtTarget(this.gazeY) &&
      springAtTarget(this.squash, SPRING_UNIT_AT, SPRING_UNIT_V) &&
      springAtTarget(this.blink, SPRING_UNIT_AT, SPRING_UNIT_V) &&
      springAtTarget(this.boost, SPRING_UNIT_AT, SPRING_UNIT_V) &&
      this.winks.every(wink => springAtTarget(wink, SPRING_UNIT_AT, SPRING_UNIT_V)) &&
      springAtTarget(this.bounce) && this.face.settled() && this.body.settled()
    )
  }

  /** 戳和 nudge 的倾斜、眼神、表情到点后收回，不再排下一轮。 */
  private releaseIdleHolds(now: number) {
    if (this.mood !== 'idle') return
    if (now >= this.ctx.biasUntil) {
      this.ctx.bias = { spin: 0, tx: 0, ty: 0, squash: 1 }
      this.ctx.biasUntil = Infinity
    }
    if (now >= this.gazeUntil) {
      this.gazeX.t = REST_GAZE.x
      this.gazeY.t = REST_GAZE.y
      this.gazeUntil = Infinity
    }
    if (now >= this.faceUntil) {
      const rest = FACE_PLAY.idle[0]
      if (this.face.key !== rest) this.face.retarget(rest, facePoints(rest))
      this.faceUntil = Infinity
    }
  }

  private finishIdlePose(now: number) {
    if (this.mood !== 'idle' || this.reduced) return
    if (this.blinkQ.length > 0) return
    if (now < this.winkAt + this.winkDur) return
    if (!springAtTarget(this.bounce)) return
    if (now < this.ctx.shakeUntil) return
    if (this.pokeShape && now < this.pokeShapeUntil + 600) return
    if (!this.springsAtTarget()) return
    snapSpring(this.spin)
    snapSpring(this.tx)
    snapSpring(this.ty)
    snapSpring(this.squash)
    snapSpring(this.blink)
    this.winks.forEach(snapSpring)
    snapSpring(this.gazeX)
    snapSpring(this.gazeY)
    snapSpring(this.boost)
    snapSpring(this.bounce)
    this.face.finish()
    this.body.finish()
  }

  private rand(a: number, b: number) {
    return a + this.random() * (b - a)
  }

  private span(a: number, b: number) {
    return spanMs(a, b, this.random())
  }

  private sign() {
    return this.random() < 0.5 ? -1 : 1
  }
}

export const BLOB_REST = new KivioBlobSim({ random: () => 0.5, reducedMotion: true }).sample(0)
