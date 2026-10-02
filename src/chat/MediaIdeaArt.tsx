// 媒体站灵感卡片的线稿插画（80×50 画布）。全部手绘 SVG、单色 currentColor，跟随主题：
// 主线 1.4、细线 0.9 半透明、面只铺一层很淡的同色。新增时保持这三种笔触，别引入彩色或图标库。
import type { ReactNode } from 'react'

export type IdeaArtName = 'vase' | 'coast' | 'poster' | 'portrait' | 'coffee' | 'arches' | 'room' | 'shanshui' | 'interior' | 'cat' | 'leaf' | 'street' | 'skyline' | 'turntable' | 'peaks' | 'fox' | 'waves' | 'road' | 'pour' | 'blossom' | 'walk' | 'rainwin' | 'fireworks' | 'plane'

const main = { strokeWidth: 1.4 } as const
const fine = { strokeWidth: 0.9, opacity: 0.6 } as const
const soft = { fill: 'currentColor', fillOpacity: 0.08, stroke: 'none' } as const
const softMain = { fill: 'currentColor', fillOpacity: 0.08, strokeWidth: 1.4 } as const
const solid = { fill: 'currentColor', stroke: 'none' } as const

const ART: Record<IdeaArtName, ReactNode> = {
  vase: <>
    <path d="M9 5 H25 L19 41 H3 Z" {...soft} />
    <path d="M17 5 L11 41 M4 23 H22" {...fine} />
    <path d="M4 41 H76" {...main} />
    <path d="M46 41 C53 38.5 61 38.5 68 41" {...fine} />
    <path d="M35 41 C30.5 36 30.5 29 34.5 25.5 V21 H45.5 V25.5 C49.5 29 49.5 36 45 41 Z" {...softMain} />
    <path d="M33 31 H47" {...fine} />
    <path d="M36.5 30.5 C36 33.5 36.6 36.5 38 38.5" {...fine} />
    <path d="M37.5 21 C36.5 15 34 11.5 30.5 9.5 M42.5 21 C43.5 14.5 46.5 11 50.5 9 M40 21 V8" {...main} />
    <path d="M33.5 13 C30.5 12 28.5 14.5 30 16.5 C32.5 16.5 34 15 33.5 13 Z" {...main} />
    <path d="M46.5 12.5 C49.5 12 51.5 14 50 16 C47.5 16 46 14.5 46.5 12.5 Z" {...main} />
    <circle cx={40} cy={6} r={2.2} {...softMain} />
  </>,
  coast: <>
    <rect x={4} y={4} width={72} height={42} rx={1.5} {...fine} />
    <rect x={4} y={4} width={72} height={5} rx={0} {...solid} />
    <rect x={4} y={41} width={72} height={5} rx={0} {...solid} />
    <path d="M28 30 A12 12 0 0 1 52 30" {...softMain} />
    <path d="M6 30 H74" {...main} />
    <path d="M60 30 L63.5 24.5 L68 23.5 L72 26 L76 30 Z" {...soft} />
    <path d="M60 30 L63.5 24.5 L68 23.5 L72 26 L76 30" {...main} />
    <path d="M35 33 H45 M36.5 35.5 H43.5 M38 38 H42" {...fine} />
    <path d="M10 34 H22 M54 34 H66 M14 37.5 H26 M56 37.5 H64" {...fine} />
    <path d="M17 16 q1.6 -1.6 3.2 0 q1.6 -1.6 3.2 0 M25 20 q1.2 -1.2 2.4 0 q1.2 -1.2 2.4 0" {...fine} />
  </>,
  poster: <>
    <rect x={22} y={3} width={36} height={44} rx={1.5} {...main} />
    <rect x={24.5} y={5.5} width={31} height={30} rx={0.5} {...fine} />
    <circle cx={48} cy={13} r={4} {...softMain} />
    <path d="M30 10 h0.01 M35 8 h0.01 M28 16 h0.01" strokeWidth={1.6} />
    <path d="M24.5 35.5 L30 24 L34 30 L39.5 18 L46 30 L49.5 25 L55.5 35.5 Z" {...soft} />
    <path d="M24.5 35.5 L30 24 L34 30 L39.5 18 L46 30 L49.5 25 L55.5 35.5" {...main} />
    <path d="M30 24 V35.5 M39.5 18 V35.5 M49.5 25 V35.5" {...fine} />
    <path d="M24.5 35.5 H55.5" {...main} />
    <path d="M31 40.5 H49" {...main} />
    <path d="M35 43.5 H45" {...fine} />
  </>,
  portrait: <>
    <rect x={7} y={5} width={22} height={33} rx={1} {...fine} />
    <path d="M18 5 V38 M7 21.5 H29" {...fine} />
    <path d="M29 9 L44 15 L44 38 L29 38 Z" {...soft} />
    <path d="M52 12.5 A6.5 6.5 0 0 1 52 25.5 Z" {...soft} />
    <circle cx={52} cy={19} r={6.5} {...main} />
    <path d="M45.6 17.5 C46 13 48.5 11.8 52.5 11.8 C55.5 11.8 57.6 13.2 58.4 15.6" {...main} />
    <path d="M49.5 25.5 V29 M54.5 25.5 V29" {...main} />
    <path d="M38 44 C38 35 44.5 30 52 30 C59.5 30 66 35 66 44" {...main} />
    <path d="M47 30.5 L52 35 L57 30.5" {...fine} />
  </>,
  coffee: <>
    <path d="M4 7 H22 M4 11 H14 M58 43 H76 M66 39 H76" {...fine} />
    <circle cx={40} cy={25} r={17} {...main} />
    <circle cx={40} cy={25} r={14.5} {...fine} />
    <circle cx={40} cy={25} r={10.5} {...main} />
    <circle cx={40} cy={25} r={8.5} {...softMain} />
    <path d="M40 30 C35.5 26.5 36 21.5 38.6 21.5 C39.6 21.5 40 22.3 40 23 C40 22.3 40.4 21.5 41.4 21.5 C44 21.5 44.5 26.5 40 30 Z" {...main} />
    <rect x={50} y={23} width={6} height={4} rx={2} {...main} />
    <path d="M12 39 L22.5 32.5" {...main} />
    <ellipse cx={25} cy={31} rx={2.6} ry={1.8} transform="rotate(-32 25 31)" {...main} />
  </>,
  arches: <>
    <path d="M7 42 V13 H73 V42" {...main} />
    <path d="M7 17 H73" {...fine} />
    <path d="M14 42 V25 A6 6 0 0 1 26 25 V42 M34 42 V25 A6 6 0 0 1 46 25 V42 M54 42 V25 A6 6 0 0 1 66 25 V42" {...main} />
    <path d="M3 42 H77" {...main} />
    <path d="M14 42 H26 L31 47 H19 Z" {...soft} />
    <path d="M34 42 H46 L51 47 H39 Z" {...soft} />
    <path d="M54 42 H66 L71 47 H59 Z" {...soft} />
    <circle cx={41.5} cy={32.2} r={1.3} {...solid} />
    <path d="M41.5 34 V38.5 M41.5 35.5 L39.8 37.5 M41.5 35.5 L43 37.2 M41.5 38.5 L40.3 42 M41.5 38.5 L42.8 42" {...main} />
  </>,
  room: <>
    <path d="M18 32 V14 L40 2 V20 Z" {...soft} />
    <path d="M18 32 V14 L40 2 L62 14 V32 L40 44 Z M40 2 V20 M18 32 L40 20 L62 32" {...main} />
    <path d="M45.5 9 L55.4 14.4 V21.4 L45.5 16 Z" {...main} />
    <path d="M50.5 11.7 V18.7 M45.5 12.5 L55.4 17.9" {...fine} />
    <path d="M40 38.5 L29 32.5 L40 26.5 L51 32.5 Z" {...fine} />
    <path d="M24 18 L34 12.5 M24 22 L34 16.5" {...main} />
    <path d="M26 16.8 V20.8 M29 15.2 V19.2 M31.5 13.8 V17.8" {...fine} />
    <path d="M47 33 L48 37 H52 L53 33 Z" {...softMain} />
    <path d="M50 33 C49 29 46 27.5 44 28 M50 33 C51 28.5 54 27 56 28 M50 33 V27" {...main} />
  </>,
  shanshui: <>
    <circle cx={62} cy={10} r={4} {...softMain} />
    <path d="M4 34 C12 24 18 19 24 22 C29 13 35 11 41 19 C47 14 55 16 61 24 C67 20 72 24 76 30 V34 Z" {...soft} />
    <path d="M4 34 C12 24 18 19 24 22 C29 13 35 11 41 19 C47 14 55 16 61 24 C67 20 72 24 76 30" {...fine} />
    <path d="M4 41 C10 32 16 29 22 33 C27 27 33 27 38 35 C42 33 45 34 48 38 L52 41 Z" {...softMain} />
    <path d="M8 30 H20 M44 27 H60 M52 31 H70" {...fine} />
    <path d="M4 41 H76" {...main} />
    <path d="M56 39.5 Q60.5 42 65 39.5 Z" {...main} />
    <path d="M61 39.5 V36.5 M59.8 37.6 H62.2" {...main} />
    <path d="M30 44 H50 M58 44.5 H70" {...fine} />
  </>,
  interior: <>
    <rect x={30} y={6} width={18} height={12} rx={0.5} {...fine} />
    <path d="M32 16 L37 11 L40 14 L43 10 L46 16" {...fine} />
    <path d="M4 42 H76" {...main} />
    <path d="M21 36 V27 C21 25 22.5 24 24.5 24 H55.5 C57.5 24 59 25 59 27 V36" {...softMain} />
    <path d="M17 30 H22 V38 H17 Z M58 30 H63 V38 H58 Z" {...main} />
    <path d="M17 38 H63" {...main} />
    <path d="M40 25 V36 M22 33 H58" {...fine} />
    <path d="M20 38 V42 M60 38 V42" {...main} />
    <path d="M70 42 V14 M67 42 H73" {...main} />
    <path d="M66 14 L67.5 8 H72.5 L74 14 Z" {...softMain} />
    <path d="M7 42 L8 36 H14 L15 36 Z" {...softMain} />
    <path d="M11 36 C10 30 7 27 4.5 27 M11 36 C12 30 15 28 17 28.5 M11 36 V26" {...main} />
  </>,
  cat: <>
    <path d="M8 43 H72" {...fine} />
    <path d="M28 38 C26 30 31 24.5 40 24.5 C49.5 24.5 55.5 30 53.5 37 C52.5 40.5 48 42 40 42 H31.5 C29.5 42 28.5 40.5 28 38 Z" {...softMain} />
    <path d="M35 28 C38 30 39 33.5 38.5 37 M44 27 C46.5 29.5 47.5 33 47 36" {...fine} />
    <path d="M53.5 37 C55 43 42 44.5 33 41.8" {...main} />
    <path d="M25 40.5 C21.5 40.5 20.5 37.5 22 35.5 L21.5 30.5 L25 33.2 C26.6 32.6 28.6 32.6 30 33.2 L33.2 30.5 L33 35.5 C34.2 37.5 32.7 40.5 29.5 40.5 Z" {...main} />
    <path d="M23.5 36.6 q1 0.9 2 0 M28 36.6 q1 0.9 2 0" {...fine} />
    <path d="M59 15 h3.5 l-3.5 3.5 h3.5 M64.5 9 h2.5 l-2.5 2.5 h2.5" {...fine} />
  </>,
  leaf: <>
    <rect x={56} y={36} width={18} height={9} rx={1} {...fine} />
    <path d="M59 39.5 H71 M59 42 H67" {...fine} />
    <path d="M38 45 C21 41 15 27 23 13 C29 6 42 4 50 10 C60 18 56 37 38 45 Z" {...softMain} />
    <path d="M38 45 C40 33 40 21 37 9" {...fine} />
    <path d="M39.5 37 C33 36 27 33 23 30 M39.7 31 C33.5 28.5 28 24.5 25 20.5 M39.4 24 C34 21 30.5 17 29 13 M39.7 36 C45 34.5 49 31 51.5 27 M39.9 29.5 C45 27 48.5 22.5 50 18 M39.4 22.5 C43 19.5 45 15.5 45.5 11" {...fine} />
    <path d="M38 45 L34 48" {...main} />
  </>,
  street: <>
    <path d="M50 46 V12 H62 V20 H72 V46 Z" {...soft} />
    <path d="M50 46 V12 H62 V20 H72 V46" {...fine} />
    <rect x={53} y={15} width={7} height={4} rx={0.8} {...main} />
    <path d="M64 24 h3 M64 28 h3 M64 32 h3 M53 24 h3 M53 28 h3" {...fine} />
    <path d="M16 44 V12 C16 8.5 18 7 21.5 7 H25" {...main} />
    <path d="M23.5 7 H28.5 L27.5 10 H24.5 Z" {...softMain} />
    <path d="M24 12 L22 20 M28 12 L30 20" {...fine} opacity={0.35} />
    <path d="M8 6 l-1.5 4 M12 14 l-1.5 4 M33 5 l-1.5 4 M38 15 l-1.5 4 M44 7 l-1.5 4 M30 24 l-1.5 4 M8 26 l-1.5 4 M42 26 l-1.5 4" {...fine} />
    <path d="M30 28 A8 5.5 0 0 1 46 28 Z" {...softMain} />
    <path d="M38 28 V36.5 q0 1.5 -1.4 1.5" {...main} />
    <circle cx={38} cy={31} r={1.5} {...main} />
    <path d="M38 32.5 V38 M36.5 38 L35.5 43 M39.5 38 L40.5 43" {...main} />
    <path d="M4 44 H76" {...main} />
    <path d="M10 46.5 H24 M34 46.5 H42 M54 46.5 H66" {...fine} />
  </>,
  skyline: <>
    <circle cx={14} cy={12} r={4} {...softMain} />
    <path d="M8 42 V31 H14 V25 H20 V35 H25 V19 H32 V28 H38 V23 H45 V32 H51 V16 H58 V27 H64 V34 H72 V42 Z" {...soft} />
    <path d="M8 42 V31 H14 V25 H20 V35 H25 V19 H32 V42 M32 28 H38 V42 M38 23 H45 V42 M45 32 H51 V16 H58 V42 M58 27 H64 V42 M64 34 H72 V42" {...main} />
    <path d="M27.5 23 h0.01 M29.5 27 h0.01 M53 20 h0.01 M55.5 24 h0.01 M41 27 h0.01 M16.5 29 h0.01" strokeWidth={1.5} />
    <path d="M4 42 H76" {...main} />
    <path d="M30 6 H26 V10 M50 6 H54 V10" {...fine} />
    <path d="M40 4 V7" {...fine} />
  </>,
  turntable: <>
    <ellipse cx={40} cy={41} rx={24} ry={5} {...main} />
    <ellipse cx={40} cy={41} rx={17} ry={3} {...fine} />
    <path d="M23 35 V25 C28 25 31.5 19.5 36.5 18.5 L43 27 C50 27.5 56 29 57 35 Z" {...softMain} />
    <path d="M23 35 H57" {...main} />
    <path d="M23 32 H57" {...fine} />
    <path d="M33 21 L36.5 26.5 M37 19.5 L40.5 25" {...fine} />
    <path d="M12 16 A28 9 0 0 1 68 16" {...fine} />
    <path d="M65 12.5 L68 16 L63.5 17" {...fine} />
  </>,
  peaks: <>
    <circle cx={60} cy={13} r={4.5} {...softMain} />
    <path d="M66.0 15.5 L67.9 16.3 M62.5 19.0 L63.3 20.9 M57.5 19.0 L56.7 20.9 M54.0 15.5 L52.1 16.3 M54.0 10.5 L52.1 9.7 M57.5 7.0 L56.7 5.1 M62.5 7.0 L63.3 5.1 M66.0 10.5 L67.9 9.7" {...fine} />
    <path d="M6 42 L24 18 L33 29 L44 13 L72 42 Z" {...soft} />
    <path d="M6 42 L24 18 L33 29 L44 13 L72 42" {...main} />
    <path d="M20.5 22.5 L22.5 23.5 L24 21.5 L25.5 23.5 L27.5 22 M40.5 17.5 L42.5 19 L44 17 L46 19 L48 18" {...fine} />
    <path d="M8 33 C12 31 18 31 24 33 M34 36 C42 34 50 34 58 36 M48 30 C54 28.5 60 28.5 66 30" {...fine} />
    <path d="M4 42 H76" {...main} />
  </>,
  fox: <>
    <path d="M66 44 L70 31 L74 44 Z M60 44 L63 35 L66 44" {...fine} />
    <path d="M70 44 V46" {...fine} />
    <path d="M8 40 Q14 30 22 33" strokeDasharray="1.5 3" {...fine} />
    <path d="M43 37.5 C38.5 40 38.5 44 42 44 H52 C55.5 44 55.5 40 51 37.5" {...softMain} />
    <path d="M39.5 42.5 C32 44.5 25.5 40.5 27.5 33 C29 37 33 39.2 38.5 38.4" {...softMain} />
    <path d="M27.5 33 C28.5 35.2 30 36.2 31.6 36.5" {...fine} />
    <path d="M36 16 L39 25 M58 16 L55 25 M36 16 L42 21 H52 L58 16" {...main} />
    <path d="M39 25 C39 31 42.5 36 47 38 C51.5 36 55 31 55 25" {...main} />
    <circle cx={43.5} cy={27} r={0.9} {...solid} />
    <circle cx={50.5} cy={27} r={0.9} {...solid} />
    <path d="M46 32.5 H48" {...main} />
    <path d="M6 44 H76" {...main} />
  </>,
  waves: <>
    <path d="M6 42 C16 41 22 32 30 25 C38 18 50 15 56 21 C62 30 68 34 76 35 V44 H6 Z" {...soft} />
    <path d="M6 42 C16 41 22 32 30 25 C38 18 50 15 56 21 C60 25 57 31 51 30 C47 29.5 46 25.5 50 24.5" {...main} />
    <path d="M56 21 C62 30 68 34 76 35" {...main} />
    <path d="M18 39 C24 35 28 31 33 28 M26 41 C32 37 38 33 44 31" {...fine} />
    <circle cx={60} cy={15} r={0.8} {...fine} />
    <circle cx={63} cy={18} r={0.6} {...fine} />
    <circle cx={57.5} cy={12.5} r={0.6} {...fine} />
    <circle cx={64} cy={13} r={0.5} {...fine} />
    <path d="M8 18 H18 M10 22 H16 M6 26 H13" {...fine} />
    <path d="M4 44 H76" {...main} />
  </>,
  road: <>
    <path d="M34 18 A6 6 0 0 1 46 18 Z" {...softMain} />
    <path d="M4 18 H76" {...main} />
    <path d="M8 46 L37 18 M72 46 L43 18" {...main} />
    <path d="M40 21 V30" strokeDasharray="2 2.5" {...fine} />
    <path d="M12 34 L20 26 M68 34 L60 26 M8 40 L13 35 M72 40 L67 35" {...fine} />
    <path d="M30.5 41 V35 C30.5 33.5 31.5 32.5 33 32.5 H47 C48.5 32.5 49.5 33.5 49.5 35 V41 Z" {...softMain} />
    <path d="M33 32.5 L35 27.5 H45 L47 32.5" {...main} />
    <path d="M35.8 28.5 H44.2 L45.6 32.5" {...fine} />
    <rect x={32} y={35.5} width={4} height={1.6} rx={0.6} {...solid} />
    <rect x={44} y={35.5} width={4} height={1.6} rx={0.6} {...solid} />
    <path d="M31.5 41 V43 H34.5 V41 M45.5 41 V43 H48.5 V41" {...main} />
  </>,
  pour: <>
    <path d="M55 3 L47 10.5 L49.5 13 L58 5.5 Z" {...softMain} />
    <path d="M47.8 11.8 C44.5 17 41.5 23 40.5 31" {...main} />
    <path d="M31.3 30 L33 44 H47 L48.7 30 Z" {...soft} />
    <path d="M30 18 L33 44 H47 L50 18" {...main} />
    <path d="M31.3 30 H48.7" {...main} />
    <path d="M35 34.5 l3 -1 l1 3 l-3 1 Z M41.5 38 l2.8 0.6 l-0.6 2.8 l-2.8 -0.6 Z" {...fine} />
    <circle cx={37} cy={27.5} r={0.7} {...fine} />
    <circle cx={44} cy={27} r={0.6} {...fine} />
    <circle cx={39} cy={25} r={0.5} {...fine} />
    <circle cx={43.5} cy={24.5} r={0.5} {...fine} />
    <path d="M44.5 41 v-1.5 M38 41.5 v-1.5 M42 36 v-1.5" {...fine} />
    <path d="M22 44 H58" {...main} />
  </>,
  blossom: <>
    <path d="M10 44 H70" {...main} />
    <path d="M18 44 V28 M40 44 V28 M62 44 V25" {...main} />
    <path d="M18 38 C15 36.5 13 34 13 32 C15.5 32.5 17.5 34.5 18 38 M40 39 C43 37.5 45 35 45 33 C42.5 33.5 40.5 35.5 40 39 M62 38 C59 36.5 57 34 57 32 C59.5 32.5 61.5 34.5 62 38" {...fine} />
    <path d="M18 28 C15 24 16 19 18 17 C20 19 21 24 18 28 Z" {...softMain} />
    <path d="M40 28 C35.5 26.5 34 21 35.5 17 C37.5 20.5 39 23.5 40 28 Z" {...softMain} />
    <path d="M40 28 C44.5 26.5 46 21 44.5 17 C42.5 20.5 41 23.5 40 28 Z" {...softMain} />
    <path d="M40 28 C38.5 24 38.5 19.5 40 16 C41.5 19.5 41.5 24 40 28 Z" {...main} />
    <circle cx={62.0} cy={15.4} r={3} {...softMain} />
    <circle cx={66.37} cy={18.58} r={3} {...softMain} />
    <circle cx={64.7} cy={23.72} r={3} {...softMain} />
    <circle cx={59.3} cy={23.72} r={3} {...softMain} />
    <circle cx={57.63} cy={18.58} r={3} {...softMain} />
    <circle cx={62} cy={20} r={2} {...main} />
    <path d="M24 23 H31 M29 21.5 L31 23 L29 24.5 M47 23 H54 M52 21.5 L54 23 L52 24.5" {...fine} />
  </>,
  walk: <>
    <path d="M18 7 H14 V11 M62 7 H66 V11" {...fine} />
    <path d="M60 4 l-1 3 M68 12 l-1.5 2 M56 18 l-1 2.5 M12 22 l1.5 2.5 M66 26 l-1 2" {...fine} />
    <circle cx={41} cy={12} r={3.2} {...softMain} />
    <path d="M41 15.2 C40.2 19 40 23 39.6 27.5" {...main} />
    <path d="M40.6 18.5 L36.5 24.5 M40.6 18.5 L45 23.5" {...main} />
    <path d="M39.6 27.5 L35 36.5 M39.6 27.5 L43.5 33 L45.5 37" {...main} />
    <path d="M23 17 H31 M21 23 H31 M24 29 H32" {...fine} />
    <path d="M8 38 H72" {...main} />
    <path d="M12 41 H20 M28 41 H36 M44 41 H52 M60 41 H68" {...fine} />
  </>,
  rainwin: <>
    <circle cx={26} cy={16} r={5} {...soft} />
    <circle cx={52} cy={22} r={7} {...soft} />
    <circle cx={38} cy={33} r={4} {...soft} />
    <circle cx={62} cy={36} r={4.5} {...soft} />
    <circle cx={20} cy={34} r={3.5} {...soft} />
    <rect x={12} y={4} width={56} height={42} rx={1.5} {...main} />
    <path d="M40 4 V46 M12 25 H68" {...main} />
    <path d="M22 10 C20.8 11.6 20.8 13 22 13 C23.2 13 23.2 11.6 22 10 Z" {...softMain} />
    <path d="M49 30 C47.8 31.6 47.8 33 49 33 C50.2 33 50.2 31.6 49 30 Z" {...softMain} />
    <path d="M58 9 C57 10.4 57 11.6 58 11.6 C59 11.6 59 10.4 58 9 Z" {...softMain} />
    <path d="M22 13 C22.5 16 21.5 19 22.3 22 M49 33 C49.6 36 48.6 39 49.4 43 M58 11.6 C58.4 14 57.6 17 58.2 21" {...fine} />
    <path d="M31 8 v2 M34 15 v1.5 M17 29 v2 M62 30 v1.5 M45 9 v1.5 M30 38 v2" {...fine} />
  </>,
  fireworks: <>
    <path d="M30.5 15.0 L38.0 15.0 M30.2 16.2 L36.7 20.0 M29.2 17.2 L33.0 23.7 M28.0 17.5 L28.0 25.0 M26.8 17.2 L23.0 23.7 M25.8 16.2 L19.3 20.0 M25.5 15.0 L18.0 15.0 M25.8 13.8 L19.3 10.0 M26.8 12.8 L23.0 6.3 M28.0 12.5 L28.0 5.0 M29.2 12.8 L33.0 6.3 M30.2 13.7 L36.7 10.0" {...main} />
    <path d="M39.1 18.0 L40.6 18.4 M36.1 23.1 L37.2 24.2 M31.0 26.1 L31.4 27.6 M25.0 26.1 L24.6 27.6 M19.9 23.1 L18.8 24.2 M16.9 18.0 L15.4 18.4 M16.9 12.0 L15.4 11.6 M19.9 6.9 L18.8 5.8 M25.0 3.9 L24.6 2.4 M31.0 3.9 L31.4 2.4 M36.1 6.9 L37.2 5.8 M39.1 12.0 L40.6 11.6" {...fine} />
    <path d="M57.9 11.6 L62.7 13.2 M57.2 12.6 L60.1 16.7 M56.0 13.0 L56.0 18.0 M54.8 12.6 L51.9 16.7 M54.1 11.6 L49.3 13.2 M54.1 10.4 L49.3 8.8 M54.8 9.4 L51.9 5.3 M56.0 9.0 L56.0 4.0 M57.2 9.4 L60.1 5.3 M57.9 10.4 L62.7 8.8" {...main} />
    <circle cx={56} cy={11} r={0.9} {...solid} />
    <circle cx={28} cy={15} r={1} {...solid} />
    <path d="M28 26 V38 M56 19 V36" strokeDasharray="1 2.5" {...fine} />
    <path d="M4 46 V36 H10 V31 H16 V38 H22 V33 H30 V40 H36 V30 H42 V36 H50 V32 H58 V38 H64 V29 H70 V35 H76 V46 Z" {...soft} />
    <path d="M4 46 V36 H10 V31 H16 V38 H22 V33 H30 V40 H36 V30 H42 V36 H50 V32 H58 V38 H64 V29 H70 V35 H76 V46" {...main} />
  </>,
  plane: <>
    <path d="M10 14 C10 11 14 10 15.5 12 C16.5 9 21.5 9 22 12.5 C24.5 12 25.5 15 23.5 16 H11 C9.5 16 9.5 14 10 14 Z" {...fine} />
    <path d="M56 38 C56 35.5 59.5 34.5 61 36.5 C62 34 66.5 34 67 37 C69 36.5 70 39 68.5 40 H57 C55.5 40 55.5 38 56 38 Z" {...fine} />
    <path d="M6 42 C16 40 18 30 24 30 C30 30 28 38 22 37 C18 36.5 22 28 34 25" strokeDasharray="1.5 3" {...fine} />
    <path d="M36 25 L66 12 L52 33 Z" {...softMain} />
    <path d="M66 12 L45.5 27.5 L44 36 L49.5 30.5" {...main} />
    <path d="M45.5 27.5 L52 33" {...fine} />
  </>,
}

export function IdeaArt({ name }: { name: IdeaArtName }) {
  return (
    <svg viewBox="0 0 80 50" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {ART[name]}
    </svg>
  )
}
