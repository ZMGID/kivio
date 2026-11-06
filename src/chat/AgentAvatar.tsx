import { memo } from 'react'
import { UserRound } from 'lucide-react'
import { BLOB_BLUE } from './kivioBlobSim'

const round = 'M12 2a10 10 0 1 1 0 20 10 10 0 0 1 0-20Z'
const silhouettes = [
  round,
  'M3 8a4 4 0 0 1 3-4l10-2a4 4 0 0 1 5 3l2 10a4 4 0 0 1-3 5l-10 2a4 4 0 0 1-5-3Z',
  'M3 13C2 7 6 4 11 3c6-2 11 1 11 7 1 6-2 10-8 11C8 23 3 20 3 13Z',
  'M2 13C2 9 4 7 7 6c2-4 6-5 10-3 4 1 6 4 5 8 2 4-1 8-5 9-3 3-7 2-10 0-4 0-6-3-5-7Z',
  'M9 3c1-2 4-2 6 0l7 10c2 3 1 5-2 6L7 22c-4 1-6-1-5-5Z',
  'M10 2c3-1 5 1 7 3 4 0 6 2 5 6 1 3 0 6-4 7-2 4-5 5-8 3-4 1-7-1-7-5-2-3-1-6 2-8 0-3 2-5 5-6Z',
  'M7 4c3-2 9-3 12 0l3 5c2 3 1 6-1 8l-5 4c-2 2-5 2-8 0l-4-4c-2-2-3-5-1-8Z',
  'M2 11c0-5 5-8 11-8 7 0 10 4 10 9 0 6-6 9-12 9-6 0-9-4-9-10Z',
]
const palette = ['#3b60c7', '#794ac3', '#a13b79', '#b64939', '#a16825', '#238260', '#247e9f', '#576480']

export const AgentAvatar = memo(function AgentAvatar({ agentId }: { agentId: string }) {
  if (agentId === '@user') return <UserRound size={16} className="shrink-0" aria-hidden="true" />
  const main = agentId.toLowerCase() === 'main'
  let hash = 2166136261
  for (const char of agentId) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619) >>> 0
  const silhouette = main ? round : silhouettes[(hash >>> 16) % silhouettes.length]
  const color = main ? BLOB_BLUE : palette[(hash >>> 21) % palette.length]
  return <svg aria-hidden="true" width={20} height={20} viewBox="0 0 24 24" className="shrink-0">
    <path d={silhouette} fill={color} />
    <g fill="white">
      <rect x="9.5" y="7.5" width="3.4" height="6.4" rx="1.7" transform="rotate(-34 11.2 10.7)" />
      <rect x="15" y="5.2" width="2.2" height="4.8" rx="1.1" transform="rotate(-34 16.1 7.6)" />
    </g>
  </svg>
})
