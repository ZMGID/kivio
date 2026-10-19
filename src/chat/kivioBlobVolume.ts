/** A soft volume projected with an orthographic camera. No lighting or WebGL.
 * The equatorial profile and its depth define ONE surface for both silhouette
 * and facial decals. Expressions are applied in material space before rotation.
 */
import { BODY_R, CX, CY, polyPath, type Pt } from './kivioBlobShapes'

type Vec3 = [number, number, number]
export interface VolumePose {
  yaw: number
  pitch: number
  squash: number
}

const LATITUDES = 16
const RINGS = Array.from({ length: LATITUDES - 1 }, (_, i) => {
  const a = -Math.PI / 2 + (i + 1) * Math.PI / LATITUDES
  return [Math.cos(a), Math.sin(a)] as const
})

function cross(a: Pt, b: Pt, c: Pt) {
  return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

function hull(points: Pt[]): Pt[] {
  points.sort((a, b) => a[0] - b[0] || a[1] - b[1])
  const lower: Pt[] = [], upper: Pt[] = []
  for (const p of points) {
    while (lower.length > 1 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) lower.pop()
    lower.push(p)
  }
  for (let i = points.length - 1; i >= 0; i--) {
    const p = points[i]
    while (upper.length > 1 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) upper.pop()
    upper.push(p)
  }
  return lower.slice(0, -1).concat(upper.slice(0, -1))
}

/** Rounded convex bodies keep a stable, readable silhouette at icon sizes.
 * Concave decorative profiles (cloud/burst) become soft volume variations.
 */
function prepareVolume(profile: Pt[]) {
  const outline = hull(profile.map(([x, y]) => [x - CX, y - CY]))
  let area = 0
  for (let i = 0; i < outline.length; i++) {
    const a = outline[i], b = outline[(i + 1) % outline.length]
    area += a[0] * b[1] - b[0] * a[1]
  }
  const xs = outline.map(p => p[0]), ys = outline.map(p => p[1])
  const hx = (Math.max(...xs) - Math.min(...xs)) / 2
  const hy = (Math.max(...ys) - Math.min(...ys)) / 2
  const centerY = (Math.max(...ys) + Math.min(...ys)) / 2
  // A soft square has rounded-square side sections too. Derive its exponent
  // continuously from the material profile, so interrupted morphs keep depth.
  const fullness = area / (8 * hx * hy)
  const exponent = 2 + 2.2 * Math.max(0, Math.min(1, (fullness - Math.PI / 4) / (0.935 - Math.PI / 4)))
  const power = 2 / exponent
  // Simpson integration of the sweep volume (16 intervals, bounded work).
  let integral = 1
  for (let i = 1; i < 16; i++) integral += (i % 2 ? 4 : 2) * Math.pow(1 - Math.pow(i / 16, exponent), power)
  const volumeFactor = integral / 24
  const depth = BODY_R * Math.PI * BODY_R * BODY_R / Math.max(area / 2, BODY_R * BODY_R) * (4 / 3) / volumeFactor
  const mesh: Vec3[] = [[0, 0, -depth], [0, 0, depth]]
  for (const [radial, z] of RINGS) {
    const r = Math.pow(radial, power)
    const d = Math.sign(z) * Math.pow(Math.abs(z), power) * depth
    for (const [x, y] of outline) mesh.push([x * r, y * r, d])
  }
  // Intersect a radial ray with the cross-section; the face remains attached
  // when circle, soft square and compressed body blend into one another.
  const radius = (x: number, y: number) => {
    const length = Math.hypot(x, y)
    if (length < 1e-8) return BODY_R
    const dx = x / length, dy = y / length
    for (let i = 0; i < outline.length; i++) {
      const a = outline[i], b = outline[(i + 1) % outline.length]
      const ex = b[0] - a[0], ey = b[1] - a[1]
      const den = dx * ey - dy * ex
      if (Math.abs(den) < 1e-8) continue
      const distance = (a[0] * ey - a[1] * ex) / den
      const along = (a[0] * dy - a[1] * dx) / den
      if (distance > 0 && along >= -1e-6 && along <= 1 + 1e-6) return distance
    }
    return BODY_R
  }
  const surface = (x: number, y: number): Vec3 => {
    x *= hx / BODY_R
    y = y * hy / BODY_R + centerY
    const rho = Math.hypot(x, y) / radius(x, y)
    return [x, y, depth * Math.pow(Math.max(0, 1 - Math.pow(rho, exponent)), 1 / exponent)]
  }
  return { mesh, surface }
}

function samePoints(a: Pt[], b: Pt[]) {
  return a.length === b.length && a.every((p, i) => p[0] === b[i][0] && p[1] === b[i][1])
}

/** One bounded cache per character, owned by its simulation. Snapshot values:
 * PolyMorph mutates its vertices in place, so identity is not an invalidation key.
 * Material geometry is prepared once and reused across camera rotations/blinks.
 */
export function createBlobProjector() {
  let profileSnapshot: Pt[] = []
  let volume: ReturnType<typeof prepareVolume> | undefined
  let eyeSnapshots: Pt[][] = []
  let liftedEyes: Vec3[][][] = []
  let lastPose: VolumePose | undefined
  let bodyD = ''
  let eyePaths: string[] = []
  return (profile: Pt[], eyes: Pt[][], pose: VolumePose) => {
    const changed = !volume || !samePoints(profile, profileSnapshot)
    if (changed) {
      profileSnapshot = profile.map(p => [...p])
      volume = prepareVolume(profile)
      eyeSnapshots = []
    }
    const { mesh, surface } = volume!
    const eyesChanged = eyes.length !== eyeSnapshots.length || eyes.some((eye, i) => !samePoints(eye, eyeSnapshots[i] ?? []))
    if (eyesChanged) {
      eyeSnapshots = eyes.map(eye => eye.map(p => [...p]))
      liftedEyes = eyes.map(eye => eye.map(([px, py]) => {
        const x = px - CX, y = py - CY, e = 0.08
        return [surface(x, y), surface(x + e, y), surface(x, y + e)]
      }))
    }
    const poseChanged = !lastPose || pose.yaw !== lastPose.yaw || pose.pitch !== lastPose.pitch || pose.squash !== lastPose.squash
    if (!changed && !eyesChanged && !poseChanged) return { bodyD, eyes: eyePaths }
    const sy = Math.max(0.72, Math.min(1.22, pose.squash))
    const sx = 1 / Math.sqrt(sy)
    const cy = Math.cos(pose.yaw), snY = Math.sin(pose.yaw)
    const cp = Math.cos(pose.pitch), snP = Math.sin(pose.pitch)
    const rotate = ([x, y, z]: Vec3): Vec3 => {
      // Deform in material coordinates, then pitch and yaw the whole volume.
      x *= sx; y *= sy; z *= sx
      const py = cp * y - snP * z, pz = snP * y + cp * z
      return [cy * x + snY * pz, py, -snY * x + cy * pz]
    }
    const screen = (p: Vec3): Pt => [CX + p[0], CY + p[1]]
    if (changed || poseChanged) bodyD = polyPath(hull(mesh.map(p => screen(rotate(p)))))
    eyePaths = liftedEyes.map(eye => {
      const vertices = eye.map(([point, da, db]) => {
        const p = rotate(point)
        // The camera-facing test uses the actual deformed surface normal,
        // including square-body curvature, rather than hiding an entire eye.
        const a = rotate(da), b = rotate(db)
        const facing = (a[0] - p[0]) * (b[1] - p[1]) - (a[1] - p[1]) * (b[0] - p[0])
        return { p, facing }
      })
      const visible: Pt[] = []
      for (let i = 0; i < vertices.length; i++) {
        const a = vertices[i], b = vertices[(i + 1) % vertices.length]
        if (a.facing >= 0) visible.push(screen(a.p))
        if ((a.facing >= 0) !== (b.facing >= 0)) {
          const t = a.facing / (a.facing - b.facing)
          visible.push(screen(a.p.map((v, j) => v + (b.p[j] - v) * t) as Vec3))
        }
      }
      return visible.length >= 3 ? polyPath(visible) : ''
    })
    lastPose = { ...pose }
    return { bodyD, eyes: eyePaths }
  }
}

/** Stateless convenience for fixed previews; live characters retain a projector. */
export function projectBlobVolume(profile: Pt[], eyes: Pt[][], pose: VolumePose) {
  return createBlobProjector()(profile, eyes, pose)
}
