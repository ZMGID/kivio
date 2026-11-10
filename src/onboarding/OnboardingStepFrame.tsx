import type { CSSProperties, ReactNode } from 'react'

type OnboardingStepFrameProps = {
  /** 章节序号，如 2 → 「02」。 */
  index: number
  eyebrow: string
  title: string
  subtitle?: string
  /** 左栏下方的动态插画，随当前设置实时变化。 */
  art?: ReactNode
  className?: string
  children: ReactNode
}

/**
 * 引导的「章节」版式：左栏是叙事（序号、标题、说明与插画，宽屏时吸顶），右栏是可滚动的配置。
 * 子元素带 `ob-in` 时按 `--i` 依次入场。
 */
export function OnboardingStepFrame({ index, eyebrow, title, subtitle, art, className, children }: OnboardingStepFrameProps) {
  return (
    <div className={['ob-chapter', className].filter(Boolean).join(' ')}>
      <section className="ob-lede">
        <div className="ob-eyebrow ob-in" style={{ '--i': 0 } as CSSProperties}>
          <span className="ob-eyebrow-num">{String(index).padStart(2, '0')}</span>
          <span className="ob-eyebrow-rule" aria-hidden />
          <span>{eyebrow}</span>
        </div>
        <h1 className="ob-title ob-in" style={{ '--i': 1 } as CSSProperties}>{title}</h1>
        {subtitle ? <p className="ob-sub ob-in" style={{ '--i': 2 } as CSSProperties}>{subtitle}</p> : null}
        {art ? <div className="ob-art ob-in" style={{ '--i': 3 } as CSSProperties} aria-hidden>{art}</div> : null}
      </section>
      <section className="ob-work">{children}</section>
    </div>
  )
}

type OnboardingBlockProps = {
  /** 入场顺序。 */
  order: number
  label?: ReactNode
  aside?: ReactNode
  className?: string
  children: ReactNode
}

/** 右栏中的一组配置：带标题的纸面卡片，按 order 错落入场。 */
export function OnboardingBlock({ order, label, aside, className, children }: OnboardingBlockProps) {
  return (
    <div className={['ob-block ob-in ob-in--work', className].filter(Boolean).join(' ')} style={{ '--i': order + 1 } as CSSProperties}>
      {label || aside ? (
        <div className="ob-block-head onboarding-section-label">
          <span>{label}</span>
          {aside ? <span className="ob-block-aside">{aside}</span> : null}
        </div>
      ) : null}
      <div className="ob-block-body">{children}</div>
    </div>
  )
}
