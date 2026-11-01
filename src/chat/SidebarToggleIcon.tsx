import { useId } from 'react'

/** 左右侧栏共用轮廓，内部隔线和色块预示面板的开合方向。 */
export function SidebarToggleIcon({ side, open }: { side: 'left' | 'right'; open: boolean }) {
  const clipId = useId()
  return (
    <svg width={15} height={15} viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <defs>
        <clipPath id={clipId}><rect x="2" y="2" width="12" height="12" rx="2.5" /></clipPath>
      </defs>
      <g transform={side === 'right' ? 'translate(16 0) scale(-1 1)' : undefined}>
        <g clipPath={`url(#${clipId})`}>
          <path
            d="M2 2h3.5v12H2Z"
            fill="currentColor"
            className={`opacity-0 transition-[opacity,transform] duration-[var(--kv-dur-normal)] ease-out group-hover:opacity-15 group-focus-visible:opacity-15 motion-reduce:transition-none ${
              open ? 'group-hover:scale-x-[0.57] group-focus-visible:scale-x-[0.57]' : 'scale-x-[0.57] group-hover:scale-x-100 group-focus-visible:scale-x-100'
            }`}
            style={{ transformBox: 'fill-box', transformOrigin: 'left center' }}
          />
        </g>
        <rect x="2" y="2" width="12" height="12" rx="2.5" stroke="currentColor" strokeWidth="1.25" />
        <path
          d="M5.5 2.5v11"
          stroke="currentColor"
          strokeWidth="1.25"
          strokeLinecap="round"
          className={`transition-transform duration-[var(--kv-dur-normal)] ease-out motion-reduce:transition-none ${
            open ? 'group-hover:-translate-x-[1.5px] group-focus-visible:-translate-x-[1.5px]' : '-translate-x-[1.5px] group-hover:translate-x-0 group-focus-visible:translate-x-0'
          }`}
        />
      </g>
    </svg>
  )
}
