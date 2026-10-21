import { describe, expect, it } from 'vitest'
import {
  blobScheduleMs,
  BLOB_BLUE,
  BLOB_IDLE_WAKE_MIN_MS,
  BLOB_POKE_RED,
  BODY_R,
  consumeBlink,
  CX,
  CY,
  EYE_POINTS,
  KivioBlobSim,
  polyPath,
  queueBlink,
  resolveBlobMood,
} from './kivioBlobSim'

function visibleSchedule(sim: KivioBlobSim, now: number) {
  return blobScheduleMs({
    reducedMotion: false,
    hidden: false,
    onScreen: true,
    highFps: sim.wantsHighFps(now),
    idleWakeMs: sim.nextIdleWakeMs(now),
  })
}

/** 按真实帧步进，直到闲置不再要 rAF、也不再有醒来时刻。 */
function driveUntilStill(sim: KivioBlobSim, start: number, limit: number) {
  const end = start + limit
  for (let t = start; t <= end; t += 16) {
    sim.sample(t)
    if (visibleSchedule(sim, t) == null) return t
  }
  return end + 1
}

describe('kivioBlobSim', () => {
  it('queueBlink 走 70ms 眯 → 150ms 过冲 → 300ms 睁开', () => {
    const q: { at: number; v: number }[] = []
    queueBlink(q, 1000, () => 1)
    expect(q.map((k) => [k.at - 1000, k.v])).toEqual([
      [0, 0.05],
      [70, 0.05],
      [150, 1.08],
      [300, 1],
    ])
    expect(consumeBlink(q, 1000)).toBe(0.05)
    expect(consumeBlink(q, 1149)).toBe(0.05)
    expect(consumeBlink(q, 1150)).toBe(1.08)
    expect(consumeBlink(q, 1300)).toBe(1)
    expect(consumeBlink(q, 2000)).toBeNull()
  })

  it('14% 连眨会再塞两帧', () => {
    const q: { at: number; v: number }[] = []
    queueBlink(q, 0, () => 0)
    expect(q).toHaveLength(6)
    expect(q[4]).toEqual({ at: 370, v: 0.05 })
    expect(q[5]).toEqual({ at: 480, v: 1 })
  })

  it.each([false, true])('默认眼睛朝右上方看（减少动态效果：%s）', (reducedMotion) => {
    const sim = new KivioBlobSim({ random: () => 0.5, reducedMotion })
    const paint = sim.sample(0)
    const parse = (d: string) =>
      [...d.matchAll(/(-?\d+\.\d+)/g)].map((m) => Number(m[1]))
    const left = parse(paint.eyes[0].d)
    const right = parse(paint.eyes[1].d)
    expect(left.length).toBe(EYE_POINTS * 2)
    expect(right.length).toBe(EYE_POINTS * 2)
    const leftXs = left.filter((_, i) => i % 2 === 0)
    const rightXs = right.filter((_, i) => i % 2 === 0)
    expect(leftXs.reduce((sum, x) => sum + x, 0) / leftXs.length).toBeGreaterThan(CX)
    expect(Math.max(...leftXs)).toBeLessThan(Math.min(...rightXs))
    for (const eye of [left, right]) {
      expect(Math.max(...eye.filter((_, i) => i % 2 === 1))).toBeLessThan(CY)
    }
    expect(Math.min(...rightXs)).toBeGreaterThan(CX)
    expect(polyPath([[1, 2], [3, 4]])).toBe('M1.00 2.00L3.00 4.00Z')
  })

  it('思考态弹簧把 spin 拉向负角', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('think', 0)
    let paint = sim.sample(0)
    for (let t = 16; t <= 800; t += 16) paint = sim.sample(t)
    expect(sim.debug().mood).toBe('think')
    expect(sim.debug().spin).toBeLessThan(-2)
    expect(paint.rig).toContain('rotate(')
  })

  it('reduced motion 钉死 pose', () => {
    const sim = new KivioBlobSim({ random: () => 0.5, reducedMotion: true })
    sim.setMood('think', 0)
    const paint = sim.sample(800)
    expect(sim.debug().spin).toBe(0)
    expect(sim.debug().blink).toBe(1)
    expect(paint.rig.startsWith('translate(0.00 0.00)')).toBe(true)
  })

  it('闲置落定后停表，生成中才持续要帧', () => {
    const sim = new KivioBlobSim({ random: () => 0.97 })
    sim.setMood('idle', 0)
    const first = sim.sample(0)
    expect(visibleSchedule(sim, 0)).toBeNull()
    for (let t = 16; t <= 8000; t += 16) {
      expect(sim.sample(t)).toEqual(first)
      expect(visibleSchedule(sim, t)).toBeNull()
    }
    expect(sim.debug().body).toBe('circle')
    expect(first.rig.startsWith('translate(0.00 0.00)')).toBe(true)

    sim.setMood('think', 8000)
    expect(sim.wantsHighFps(8000)).toBe(true)
    for (let t = 8016; t <= 9000; t += 16) {
      sim.sample(t)
      expect(sim.wantsHighFps(t)).toBe(true)
    }
  })

  it('出错间隙不钉满帧', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('error', 0)
    for (let t = 0; t <= 2500; t += 16) sim.sample(t)
    expect(sim.wantsHighFps(2500)).toBe(false)
    expect(sim.nextIdleWakeMs(2500)).toBeGreaterThan(100)
  })

  it('blobScheduleMs：隐藏/屏外/失焦/覆盖/静稳停表，有限醒来可睡，忙碌跟 vsync', () => {
    expect(blobScheduleMs({ reducedMotion: true, hidden: false, onScreen: true, highFps: true })).toBeNull()
    expect(blobScheduleMs({ reducedMotion: false, hidden: true, onScreen: true, highFps: true })).toBeNull()
    expect(blobScheduleMs({ reducedMotion: false, hidden: false, onScreen: false, highFps: true })).toBeNull()
    expect(blobScheduleMs({ reducedMotion: false, hidden: false, onScreen: true, unfocused: true, highFps: true })).toBeNull()
    expect(blobScheduleMs({ reducedMotion: false, hidden: false, onScreen: true, covered: true, highFps: true })).toBeNull()
    expect(blobScheduleMs({
      reducedMotion: false,
      hidden: false,
      onScreen: true,
      highFps: false,
    })).toBeNull()
    expect(blobScheduleMs({
      reducedMotion: false,
      hidden: false,
      onScreen: true,
      highFps: false,
      idleWakeMs: Number.POSITIVE_INFINITY,
    })).toBeNull()
    expect(blobScheduleMs({
      reducedMotion: false,
      hidden: false,
      onScreen: true,
      highFps: false,
      idleWakeMs: 9000,
    })).toBe(9000)
    expect(blobScheduleMs({
      reducedMotion: false,
      hidden: false,
      onScreen: true,
      highFps: false,
      idleWakeMs: 1,
    })).toBe(BLOB_IDLE_WAKE_MIN_MS)
    expect(blobScheduleMs({ reducedMotion: false, hidden: false, onScreen: true, highFps: true })).toBe(0)
  })

  it('poke 会眨眼并跳一下，然后回到不再排程的静稳', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('idle', 0)
    sim.sample(0)
    sim.poke(200)
    expect(sim.wantsHighFps(200)).toBe(true)
    const paint = sim.sample(280)
    expect(paint.rig.startsWith('translate(0.00 0.00)')).toBe(false)
    const settledAt = driveUntilStill(sim, 296, 20000)
    expect(settledAt).toBeLessThanOrEqual(296 + 20000)
    const settled = sim.sample(settledAt)
    expect(sim.sample(settledAt + 6000)).toEqual(settled)
    expect(settled.rig.startsWith('translate(0.00 0.00)')).toBe(true)
    expect(sim.debug().body).toBe('circle')
    expect(visibleSchedule(sim, settledAt + 6000)).toBeNull()
  })

  it('nudge 会动一下，然后回到不再排程的静稳', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('idle', 0)
    sim.sample(0)
    sim.nudge(100)
    expect(sim.wantsHighFps(100)).toBe(true)
    const settledAt = driveUntilStill(sim, 116, 8000)
    expect(settledAt).toBeLessThanOrEqual(116 + 8000)
    const settled = sim.sample(settledAt)
    expect(sim.sample(settledAt + 4000)).toEqual(settled)
    expect(visibleSchedule(sim, settledAt + 4000)).toBeNull()
  })

  it('paused idle holds expire on resume instead of stranding the pose', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    const rest = sim.sample(0)
    sim.poke(200)
    for (let t = 216; t <= 1800; t += 16) sim.sample(t)
    // No samples while hidden. A late wake must still release the old holds.
    expect(visibleSchedule(sim, 30_000)).not.toBeNull()
    const settledAt = driveUntilStill(sim, 30_000, 20_000)
    expect(settledAt).toBeLessThanOrEqual(50_000)
    expect(sim.sample(settledAt)).toEqual(rest)
    expect(visibleSchedule(sim, settledAt)).toBeNull()
  })

  it('生成结束回到闲置后有限步落定，不再排程', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('work', 0)
    for (let t = 0; t <= 800; t += 16) {
      sim.sample(t)
      expect(sim.wantsHighFps(t)).toBe(true)
    }
    sim.setMood('idle', 800)
    expect(sim.wantsHighFps(800)).toBe(true)
    const settledAt = driveUntilStill(sim, 816, 12000)
    expect(settledAt).toBeLessThanOrEqual(816 + 12000)
    const settled = sim.sample(settledAt)
    expect(sim.sample(settledAt + 8000)).toEqual(settled)
    expect(sim.debug().body).toBe('circle')
    expect(sim.debug().face).toBe('neutral')
    expect(visibleSchedule(sim, settledAt + 8000)).toBeNull()
  })

  it('连点升温变红，换文案 nudge 不脸红', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('idle', 0)
    sim.sample(0)
    expect(sim.sample(10).fill.toLowerCase()).toBe(BLOB_BLUE)
    sim.nudge(20)
    expect(sim.debug().heat).toBe(0)
    expect(sim.sample(30).fill.toLowerCase()).toBe(BLOB_BLUE)
    expect(sim.poke(100)).toBe(1)
    expect(sim.debug().heat).toBeGreaterThan(0)
    expect(sim.sample(120).fill.toLowerCase()).not.toBe(BLOB_BLUE)
    for (let i = 1; i < 6; i++) sim.poke(100 + i * 80)
    expect(sim.debug().pokes).toBe(6)
    expect(sim.debug().heat).toBe(1)
    expect(sim.sample(600).fill.toLowerCase()).toBe(BLOB_POKE_RED)
    sim.poke(6000)
    expect(sim.debug().pokes).toBe(1)
  })

  it('resolveBlobMood 跟生成阶段走', () => {
    expect(resolveBlobMood({ active: false })).toBe('idle')
    expect(resolveBlobMood({ active: false, error: true })).toBe('error')
    expect(resolveBlobMood({ active: true })).toBe('think')
    expect(resolveBlobMood({ active: true, runningToolNames: ['web_search'] })).toBe('search')
    expect(resolveBlobMood({ active: true, runningToolNames: ['read_file'] })).toBe('work')
    expect(resolveBlobMood({ active: true, contentLen: 40, reasoningStreaming: false })).toBe('speak')
    expect(resolveBlobMood({ active: true, contentLen: 40, reasoningStreaming: true })).toBe('think')
  })

  it('resolveBlobMood：等用户压过工具态，收工窗口只在非活动时生效', () => {
    expect(resolveBlobMood({ active: true, waiting: true, runningToolNames: ['ask_user'] })).toBe('wait')
    expect(resolveBlobMood({ active: false, done: true })).toBe('done')
    expect(resolveBlobMood({ active: false, done: true, error: true })).toBe('error')
    expect(resolveBlobMood({ active: true, done: true })).toBe('think')
  })

  it('默认转向后身体仍是闭合圆形轮廓', () => {
    const sim = new KivioBlobSim({ random: () => 0.5, reducedMotion: true })
    const paint = sim.sample(0)
    const nums = [...paint.bodyD.matchAll(/(-?\d+\.\d+)/g)]
    expect(paint.bodyD.startsWith('M')).toBe(true)
    expect(paint.bodyD.endsWith('Z')).toBe(true)
    expect(sim.debug().body).toBe('circle')
    // 圆：所有点到圆心距离 ≈ BODY_R。
    for (let i = 0; i < nums.length; i += 2) {
      const dx = Number(nums[i][1]) - CX
      const dy = Number(nums[i + 1][1]) - CY
      expect(Math.hypot(dx, dy)).toBeCloseTo(BODY_R, 0)
    }
  })

  it('掷中时：思考变云、干活变方、说话带尾巴的气泡；出错必摊成一滩', () => {
    const shapeAfter = (mood: 'think' | 'work' | 'speak' | 'error', random: number) => {
      const sim = new KivioBlobSim({ random: () => random })
      sim.setMood(mood, 0)
      for (let t = 0; t <= 1600; t += 16) sim.sample(t)
      return sim.debug().body
    }
    expect(shapeAfter('think', 0.1)).toBe('cloud')
    expect(shapeAfter('work', 0.1)).toBe('squircle')
    expect(shapeAfter('speak', 0.1)).toBe('bubble')
    expect(shapeAfter('error', 0.1)).toBe('puddle')
    expect(shapeAfter('error', 0.9)).toBe('puddle')
  })

  it('没掷中：生成中大多数时候就是个圆，不变云也不变气泡', () => {
    for (const mood of ['think', 'speak', 'work', 'search'] as const) {
      const sim = new KivioBlobSim({ random: () => 0.9 })
      sim.setMood(mood, 0)
      const seen = new Set<string>()
      for (let t = 0; t <= 20000; t += 50) {
        sim.sample(t)
        seen.add(sim.debug().body)
      }
      expect([...seen]).toEqual(['circle'])
    }
  })

  it('身体变形是渐进的：切心情后一帧不会直接跳成目标形状', () => {
    const sim = new KivioBlobSim({ random: () => 0.1 })
    sim.setMood('idle', 0)
    sim.sample(0)
    const circle = sim.sample(16).bodyD
    sim.setMood('work', 32)
    const mid = sim.sample(48).bodyD
    let settled = mid
    for (let t = 64; t <= 2400; t += 16) settled = sim.sample(t).bodyD
    expect(mid).not.toBe(circle)
    expect(mid).not.toBe(settled)
    expect(sim.debug().body).toBe('squircle')
  })

  it('进入各心情先摆播放列表第 0 张脸；收工是笑脸 + 蹦一下', () => {
    const face = (mood: 'think' | 'search' | 'error' | 'done' | 'wait') => {
      const sim = new KivioBlobSim({ random: () => 0.5 })
      sim.setMood(mood, 0)
      sim.sample(0)
      return sim.debug().face
    }
    expect(face('think')).toBe('lookUp')
    expect(face('search')).toBe('wide')
    expect(face('error')).toBe('dizzy')
    expect(face('done')).toBe('happy')
    expect(face('wait')).toBe('wide')
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('idle', 0)
    sim.sample(0)
    sim.setMood('done', 100)
    expect(sim.wantsHighFps(100)).toBe(true)
    const paint = sim.sample(260)
    expect(paint.rig.startsWith('translate(0.00 0.00)')).toBe(false)
  })

  it('闭着的脸不眨眼：笑弯的眼皮缩放钉在 1', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('done', 0)
    for (let t = 0; t <= 600; t += 16) sim.sample(t)
    expect(sim.debug().face).toBe('happy')
    expect(sim.debug().blink).toBeCloseTo(1, 2)
  })

  it('戳一下压扁、戳多了装方、再戳炸毛，过后变回圆', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    sim.setMood('idle', 0)
    sim.sample(0)
    sim.poke(100)
    sim.sample(116)
    expect(sim.debug().body).toBe('puddle')
    expect(sim.debug().face).toBe('flat')
    for (let i = 1; i < 4; i++) sim.poke(100 + i * 80)
    sim.sample(400)
    expect(sim.debug().body).toBe('squircle')
    for (let i = 4; i < 7; i++) sim.poke(100 + i * 80)
    sim.sample(700)
    expect(sim.debug().body).toBe('burst')
    expect(sim.debug().face).toBe('dizzy')
    for (let t = 716; t <= 3200; t += 16) sim.sample(t)
    expect(sim.debug().body).toBe('circle')
  })

  it('reduced motion 下身体永远是圆、脸不变形', () => {
    const sim = new KivioBlobSim({ random: () => 0.5, reducedMotion: true })
    sim.setMood('think', 0)
    for (let t = 0; t <= 800; t += 16) sim.sample(t)
    expect(sim.debug().body).toBe('circle')
    expect(sim.debug().face).toBe('lookUp')
  })
})

// Geometric output is the public rendering boundary; avoid inspecting springs.
function eyeCenter(paint: ReturnType<KivioBlobSim['sample']>) {
  const values = [...paint.eyes[0].d.matchAll(/-?\d+\.\d+/g)].map(m => Number(m[0]))
  return values.filter((_, i) => i % 2 === 0).reduce((a, b) => a + b, 0) / (values.length / 2)
}

describe('volume motion continuity', () => {
  it('turning accelerates, brakes, and preserves momentum when the target reverses', () => {
    const sim = new KivioBlobSim({ random: () => 0.5 })
    const initial = eyeCenter(sim.sample(0))
    sim.poke(0, -1, -0.5)
    expect(eyeCenter(sim.sample(0))).toBe(initial)
    const positions = [initial]
    for (let t = 10; t <= 200; t += 10) positions.push(eyeCenter(sim.sample(t)))
    const speeds = positions.slice(1).map((x, i) => positions[i] - x)
    expect(Math.max(...speeds)).toBeGreaterThan(speeds[0] * 2)
    const before = positions[positions.length - 1]
    sim.poke(200, 1, -0.5)
    expect(eyeCenter(sim.sample(200))).toBeCloseTo(before, 2)
    expect(eyeCenter(sim.sample(210))).toBeLessThan(before)
    for (let t = 220; t <= 1800; t += 10) sim.sample(t)
    expect(eyeCenter(sim.sample(1800))).toBeGreaterThan(initial)
    const settledAt = driveUntilStill(sim, 1810, 14000)
    expect(settledAt).toBeLessThan(15810)
    expect(eyeCenter(sim.sample(settledAt))).toBeCloseTo(initial, 2)
  })

  it('repeated pokes and state changes do not teleport the current geometry', () => {
    const sim = new KivioBlobSim({ random: () => 0.1 })
    sim.sample(0)
    sim.poke(0, 1, -1)
    for (let t = 10; t <= 160; t += 10) sim.sample(t)
    const before = sim.sample(160)
    sim.poke(160, -1, 1)
    sim.setMood('work', 160)
    const after = sim.sample(160)
    expect(after.rig).toBe(before.rig)
    expect(after.bodyD).toBe(before.bodyD)
    expect(after.eyes).toEqual(before.eyes)
  })

  it('an authored glance has matching projected positions at 24, 30, 60 and 120 fps', () => {
    const centers = [24, 30, 60, 120].map(fps => {
      const sim = new KivioBlobSim({ random: () => 0.5 })
      sim.sample(0)
      sim.nudge(0)
      for (let n = 1; n <= fps; n++) sim.sample(n * 1000 / fps)
      return eyeCenter(sim.sample(1000))
    })
    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThan(0.05)
  })
})


it('large-character idle gestures vary, sleep between actions and yield to nearby gaze', () => {
  const sim = new KivioBlobSim({ random: () => 0.5 })
  sim.setAmbientMotion(true, 0)
  const rest = sim.sample(0)
  expect(sim.wantsHighFps(0)).toBe(false)
  const wake = sim.nextIdleWakeMs(0)
  expect(wake).toBeGreaterThan(2000)
  expect(wake).toBeLessThan(5000)
  sim.sample(wake)
  for (let t = wake + 16; t <= wake + 800; t += 16) sim.sample(t)
  const gesture = sim.sample(wake + 800)
  expect(gesture.eyes).not.toEqual(rest.eyes)
  const settledAt = driveUntilStill(sim, wake + 816, 6000)
  expect(sim.wantsHighFps(settledAt)).toBe(false)
  expect(sim.nextIdleWakeMs(settledAt)).toBeGreaterThan(0)
  sim.lookAt(-0.8, 0.3, settledAt)
  for (let t = settledAt; t <= settledAt + 2000; t += 16) sim.sample(t)
  expect(sim.nextIdleWakeMs(settledAt + 2000)).toBe(Infinity)
  const held = sim.sample(settledAt + 2000)
  expect(sim.sample(settledAt + 12000).eyes).toEqual(held.eyes)
  sim.releaseLook()
  sim.sample(settledAt + 12016)
  expect(sim.nextIdleWakeMs(settledAt + 12016)).toBeGreaterThan(0)
  sim.setReducedMotion(true)
  sim.sample(settledAt + 13000)
  expect(sim.wantsHighFps(settledAt + 13000)).toBe(false)
  expect(sim.nextIdleWakeMs(settledAt + 13000)).toBe(Infinity)
})


it('reschedules idle gestures after a centered pointer leaves without needing a return animation', () => {
  const sim = new KivioBlobSim({ random: () => 0.5 })
  sim.setAmbientMotion(true, 0)
  sim.sample(0)
  sim.lookAt(0, 0, 100)
  sim.sample(100)
  expect(sim.nextIdleWakeMs(100)).toBe(Infinity)
  sim.releaseLook()
  expect(sim.wantsHighFps(100)).toBe(false)
  expect(sim.nextIdleWakeMs(100)).toBeGreaterThan(0)
  expect(sim.nextIdleWakeMs(100)).toBeLessThan(13000)
})
