import { memo } from 'react'
import { bodyPoints, facePoints, polyPath, type BodyShape, type FaceName } from './kivioBlobShapes'

const bodies: BodyShape[] = ['cloud', 'squircle', 'pebble', 'bean', 'bubble', 'puddle', 'burst', 'circle']
const faces: FaceName[] = ['dots', 'focus', 'peek', 'wide', 'smirk', 'neutral', 'happy', 'content', 'lookUp', 'tiny']

/** Derive each feature independently so identities are stable without repeating six presets. */
export const SubAgentAvatar = memo(function SubAgentAvatar({ id, status = '', size = 26 }: { id: string; status?: string; size?: number }) {
  let seed = 2166136261
  for (const char of id) seed = Math.imul(seed ^ char.charCodeAt(0), 16777619) >>> 0
  const next = () => {
    seed ^= seed << 13
    seed ^= seed >>> 17
    seed ^= seed << 5
    return (seed >>> 0) / 4294967296
  }
  const body = bodies[Math.floor(next() * bodies.length)]
  const expression = faces[Math.floor(next() * faces.length)]
  const color = `hsl(${Math.round(next() * 3600) / 10} ${48 + Math.floor(next() * 18)}% ${43 + Math.floor(next() * 12)}%)`
  const tilt = Math.round(next() * 18 - 9)
  const main = id === 'main'
  const face = status === 'interrupted' ? 'sleepy' : main ? 'dots' : expression
  return <svg aria-hidden="true" width={size} height={size} viewBox="24 24 192 192" className="shrink-0">
    <g transform={`rotate(${main ? 0 : tilt} 120 120)`}>
      <path d={polyPath(bodyPoints(main ? 'circle' : body))} fill={main ? '#1d6bf0' : color} />
      {facePoints(face).map((eye, index) => <path key={index} d={polyPath(eye)} fill="#fffaf1" />)}
    </g>
  </svg>
})
