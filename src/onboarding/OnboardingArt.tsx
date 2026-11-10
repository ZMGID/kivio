import type { OnboardingStepId } from './types'

// 背景「墨迹」层：三团强调色晕染随步骤缓慢换位，等高线细墨线持续流动，章节号作为大字水印。
// 只用主题语义色（--accent / --text），随用户主题与明暗切换。

const CONTOURS = [
  'M-40 620 C 220 520, 380 700, 640 600 S 1080 460, 1480 560',
  'M-40 660 C 240 560, 400 740, 660 640 S 1100 500, 1480 600',
  'M-40 700 C 260 600, 420 780, 680 680 S 1120 540, 1480 640',
  'M-40 740 C 280 640, 440 820, 700 720 S 1140 580, 1480 680',
  'M-40 180 C 180 260, 420 120, 700 200 S 1160 300, 1480 220',
  'M-40 220 C 200 300, 440 160, 720 240 S 1180 340, 1480 260',
]

export function OnboardingAtmosphere({ step, index }: { step: OnboardingStepId; index: number }) {
  return (
    <div className="ob-atmos" aria-hidden>
      <div className="ob-ink ob-ink--a"><span /></div>
      <div className="ob-ink ob-ink--b"><span /></div>
      <div className="ob-ink ob-ink--c"><span /></div>
      <div className="ob-grain" />
      <svg className="ob-contours" viewBox="0 0 1440 900" preserveAspectRatio="xMidYMid slice">
        {CONTOURS.map((d, i) => (
          <path key={d} d={d} style={{ animationDelay: `${i * -3.2}s` }} />
        ))}
      </svg>
      {step !== 'welcome' ? (
        <span key={step} className="ob-numeral">{String(index).padStart(2, '0')}</span>
      ) : null}
    </div>
  )
}
