import { BLOB_EYE, BLOB_REST } from './kivioBlobSim'

/** Static rendering of Kivio's existing mascot keeps large contact lists quiet. */
export function BotAvatar({ color = '#6A8FBD', size = 40 }: { color?: string; size?: number }) {
  return <svg aria-hidden="true" viewBox="35 35 170 170" width={size} height={size} className="shrink-0">
    <g transform={BLOB_REST.rig}>
      <path d={BLOB_REST.bodyD} fill={color} transform={BLOB_REST.body} />
      {BLOB_REST.eyes.map((eye, index) => <path key={index} d={eye.d} fill={BLOB_EYE} transform={eye.transform} />)}
    </g>
  </svg>
}
