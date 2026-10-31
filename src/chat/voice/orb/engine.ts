export type OrbState = 'connecting' | 'listening' | 'working' | 'composing' | 'breathing' | 'solving'

export type OrbShape = {
  size: number
  stretch: number
  ripple: number
  twist: number
  speed: number
  opacity: number
}

type OrbFrame = {
  time: number
  scale: number
  stretch: number
  ripple: number
  twist: number
  yaw: number
  tiltSin: number
  tiltCos: number
}

type OrbPoint = { x: number; y: number; depth: number }

export const ORB_STATES: Record<OrbState, OrbShape> = {
  connecting: { size: 0.97, stretch: 0.94, ripple: 0.015, twist: 0.08, speed: 0.62, opacity: 0.72 },
  listening: { size: 1, stretch: 1, ripple: 0.018, twist: 0.04, speed: 0.34, opacity: 0.84 },
  working: { size: 0.99, stretch: 0.91, ripple: 0.035, twist: 0.5, speed: 0.72, opacity: 0.9 },
  composing: { size: 1.02, stretch: 1.02, ripple: 0.045, twist: -0.18, speed: 0.55, opacity: 0.96 },
  breathing: { size: 0.92, stretch: 1, ripple: 0.008, twist: 0.02, speed: 0.17, opacity: 0.54 },
  solving: { size: 0.97, stretch: 0.95, ripple: 0.04, twist: 0.65, speed: 0.7, opacity: 0.9 },
}

export function voiceEnergy(peak: number): number {
  return Number.isFinite(peak) ? Math.min(1, Math.sqrt(Math.max(0, peak)) * 1.8) : 0
}

/** Fixed samples preserve each particle's identity as the orb changes state. */
export function createOrbCloud(count: number): Float32Array {
  const points = new Float32Array(count * 3)
  const goldenAngle = Math.PI * (3 - Math.sqrt(5))
  for (let i = 0; i < count; i++) {
    const y = 1 - 2 * (i + 0.5) / count
    const ring = Math.sqrt(1 - y * y)
    const angle = i * goldenAngle
    const variation = (i * 0.754877666) % 1
    const shell = i % 4 === 0 ? 0.74 + 0.12 * variation : 0.92 + 0.08 * variation
    points[i * 3] = Math.cos(angle) * ring * shell
    points[i * 3 + 1] = y * shell
    points[i * 3 + 2] = Math.sin(angle) * ring * shell
  }
  return points
}

const HERO_CLOUD = createOrbCloud(960)
const SMALL_CLOUD = createOrbCloud(64)

export function createOrbFrame(time: number, energy: number, shape: OrbShape): OrbFrame {
  const tilt = 0.3 + Math.sin(time * 0.23) * 0.035
  return {
    time,
    scale: 0.4 * shape.size * (1 + Math.sin(time * 0.75) * 0.012 + energy * 0.04),
    stretch: shape.stretch,
    ripple: shape.ripple + energy * 0.04,
    twist: shape.twist,
    yaw: time * 0.22,
    tiltSin: Math.sin(tilt),
    tiltCos: Math.cos(tilt),
  }
}

export function orbPoint(cloud: Float32Array, index: number, frame: OrbFrame, out: OrbPoint): void {
  const x = cloud[index * 3]
  const y = cloud[index * 3 + 1]
  const z = cloud[index * 3 + 2]
  const angle = frame.yaw + frame.twist * y
  const sin = Math.sin(angle)
  const cos = Math.cos(angle)
  const turnedX = x * cos + z * sin
  const turnedZ = z * cos - x * sin
  const vertical = y * frame.tiltCos - turnedZ * frame.tiltSin
  const depth = y * frame.tiltSin + turnedZ * frame.tiltCos
  const scale = frame.scale * (1 + frame.ripple * Math.sin(y * 6 + x * 3 + frame.time * 1.1))
  out.x = 0.5 + turnedX * scale
  out.y = 0.5 + vertical * scale * frame.stretch
  out.depth = Math.max(0, Math.min(1, (depth + 1) / 2))
}

/** The same monochrome point cloud is painted at lower density in inline indicators. */
export function paintOrb(ctx: CanvasRenderingContext2D, size: number, time: number, energy: number, shape: OrbShape, ink: string): void {
  const small = size < 80
  const cloud = small ? SMALL_CLOUD : HERO_CLOUD
  const frame = createOrbFrame(time, energy, shape)
  const point: OrbPoint = { x: 0, y: 0, depth: 0 }
  const dotSize = small ? Math.max(0.35, size * 0.017) : Math.max(0.8, size * 0.0024)
  ctx.fillStyle = ink
  for (let i = 0; i < cloud.length / 3; i++) {
    orbPoint(cloud, i, frame, point)
    ctx.globalAlpha = shape.opacity * (0.2 + point.depth * 0.7)
    ctx.beginPath()
    ctx.arc(point.x * size, point.y * size, dotSize * (0.55 + point.depth * 1.2), 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.globalAlpha = 1
}
