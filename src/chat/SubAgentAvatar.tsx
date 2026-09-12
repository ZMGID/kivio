import { memo } from 'react'
import { bodyPoints, facePoints, polyPath, type BodyShape, type FaceName } from './kivioBlobShapes'

const forms: { body: BodyShape; face: FaceName; color: string }[] = [
  { body: 'cloud', face: 'dots', color: '#298fa8' },
  { body: 'squircle', face: 'focus', color: '#8971cd' },
  { body: 'egg', face: 'peek', color: '#d078a0' },
  { body: 'bubble', face: 'wide', color: '#4885cc' },
  { body: 'puddle', face: 'smirk', color: '#b38746' },
  { body: 'burst', face: 'neutral', color: '#559b85' },
]

/** Stable identity, using Lanlan's existing silhouette and eye vocabulary. */
export const SubAgentAvatar = memo(function SubAgentAvatar({ id, status = '', size = 26 }: { id: string; status?: string; size?: number }) {
  const hash = Array.from(id).reduce((value, char) => (value * 31 + char.charCodeAt(0)) >>> 0, 0)
  const form = id === 'main' ? { body: 'circle' as const, face: 'dots' as const, color: '#1d6bf0' } : forms[hash % forms.length]
  const face = status === 'completed' ? 'happy' : status === 'failed' ? 'worry' : status === 'interrupted' ? 'sleepy' : form.face
  return <svg aria-hidden="true" width={size} height={size} viewBox="20 20 200 200" className="shrink-0">
    <path d={polyPath(bodyPoints(form.body))} fill={form.color} />
    {facePoints(face).map((eye, index) => <path key={index} d={polyPath(eye)} fill="#fffaf1" />)}
  </svg>
})
