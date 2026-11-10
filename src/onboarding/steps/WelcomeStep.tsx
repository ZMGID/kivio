import { useEffect, useRef, type CSSProperties, type PointerEvent } from 'react'
import { Check, Crop, Languages, MessageSquare, TerminalSquare } from 'lucide-react'
import type { I18n, Lang } from '../../components/i18n'
import { AgentIcon } from '../../components/AgentIcon'
import { KivioBlob } from '../../chat/public/mascot'

type WelcomeStepProps = {
  t: I18n
  lang: Lang
}

const CLI_AGENTS = [
  { id: 'claude', name: 'Claude Code' },
  { id: 'codex', name: 'Codex' },
  { id: 'gemini', name: 'Gemini CLI' },
  { id: 'opencode', name: 'OpenCode' },
  { id: 'kimi', name: 'Kimi CLI' },
  { id: 'grok', name: 'Grok CLI' },
]

const order = (i: number) => ({ '--i': i } as CSSProperties)

/**
 * 序章：左侧是墨墨与主张，右侧是四张会动的「产品切片」——Agent 执行、外部 CLI、Lens 框选、截图翻译，
 * 鼠标移动时按景深轻微视差。底部跑马灯列出其余能力。
 */
export function WelcomeStep({ t, lang }: WelcomeStepProps) {
  const heroRef = useRef<HTMLDivElement>(null)
  const frameRef = useRef(0)
  useEffect(() => () => cancelAnimationFrame(frameRef.current), [])

  // 视差只写 CSS 变量，不进 React 状态；一帧最多写一次。
  const handlePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const host = heroRef.current
    if (!host || frameRef.current) return
    const { clientX, clientY } = event
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = 0
      const rect = host.getBoundingClientRect()
      if (!rect.width || !rect.height) return
      host.style.setProperty('--mx', (((clientX - rect.left) / rect.width) * 2 - 1).toFixed(3))
      host.style.setProperty('--my', (((clientY - rect.top) / rect.height) * 2 - 1).toFixed(3))
    })
  }

  const ticker = t.onboardingWelcomeTicker.split(' · ')
  const translateFrom = lang === 'zh' ? 'Ship it before Friday.' : '周五之前上线。'
  const translateTo = lang === 'zh' ? '周五之前上线。' : 'Ship it before Friday.'

  return (
    <div className="ob-hero" ref={heroRef} onPointerMove={handlePointerMove}>
      <section className="ob-hero-copy">
        <div className="ob-greet ob-in" style={order(0)}>
          <span className="ob-greet-blob"><KivioBlob size={52} mood="speak" /></span>
          <span className="ob-greet-bubble">{t.onboardingWelcomeGreeting}</span>
        </div>
        <h1 className="ob-hero-title">
          <span className="ob-hero-line ob-in" style={order(1)}>{t.onboardingWelcomeTitleLead}</span>
          <span className="ob-hero-line ob-hero-line--ink ob-in" style={order(2)}>{t.onboardingWelcomeTitleAccent}</span>
        </h1>
        <p className="ob-sub ob-hero-sub ob-in" style={order(3)}>{t.onboardingWelcomeSubtitle}</p>
        <ol className="ob-route ob-in" style={order(4)} aria-label={t.onboardingWelcomeRouteLabel}>
          {[t.onboardingStepAppearance, t.onboardingStepProvider, t.onboardingStepWebSearch, t.onboardingStepHotkey].map((label, index) => (
            <li key={label} style={order(index)}>
              <span className="ob-route-num">{String(index + 1).padStart(2, '0')}</span>
              <span>{label}</span>
            </li>
          ))}
        </ol>
        <p className="ob-hero-note ob-in" style={order(5)}>{t.onboardingWelcomeFootnote}</p>
      </section>

      <section className="ob-collage" aria-hidden>
        <div className="ob-depth ob-depth--agent" style={order(2)}>
          <div className="ob-vig ob-vig--agent">
            <header className="ob-vig-head"><MessageSquare size={13} />Kivio Agent</header>
            <div className="ob-agent-user">{t.onboardingWelcomeDemoPrompt}</div>
            <div className="ob-agent-act">
              <span className="ob-agent-spin" />
              <Check size={12} strokeWidth={2.6} className="ob-agent-check" />
              <span>{t.onboardingWelcomeDemoActivity}</span>
            </div>
            <div className="ob-agent-lines">
              <i /><i /><i />
            </div>
          </div>
        </div>

        <div className="ob-depth ob-depth--cli" style={order(3)}>
          <div className="ob-vig ob-vig--cli">
            <header className="ob-vig-head">
              <span className="ob-cli-dots"><i /><i /><i /></span>
              <TerminalSquare size={13} />~/projects
            </header>
            <div className="ob-cli-line">
              <span className="ob-cli-prompt">❯</span>
              <span className="ob-cli-roll">
                <span className="ob-cli-track">
                  {[...CLI_AGENTS, CLI_AGENTS[0]].map((agent, index) => (
                    <span key={`${agent.id}-${index}`} className="ob-cli-agent">
                      <AgentIcon id={agent.id} size={13} />
                      {agent.name}
                    </span>
                  ))}
                </span>
              </span>
            </div>
            <div className="ob-cli-note">{t.onboardingWelcomeDemoCli}</div>
          </div>
        </div>

        <div className="ob-depth ob-depth--lens" style={order(4)}>
          <div className="ob-vig ob-vig--lens">
            <header className="ob-vig-head"><Crop size={13} />Lens</header>
            <div className="ob-lens-screen">
              <span className="ob-lens-bar" />
              <span className="ob-lens-bar ob-lens-bar--short" />
              <svg className="ob-lens-chart" viewBox="0 0 120 48" preserveAspectRatio="none">
                <path d="M2 40 C 18 38, 26 30, 38 26 S 56 6, 66 8 S 84 30, 96 34 S 112 36, 118 30" />
              </svg>
              <span className="ob-lens-select"><i /><i /><i /><i /></span>
            </div>
            <div className="ob-lens-answer">{t.onboardingWelcomeDemoLens}</div>
          </div>
        </div>

        <div className="ob-depth ob-depth--translate" style={order(5)}>
          <div className="ob-vig ob-vig--translate">
            <header className="ob-vig-head"><Languages size={13} />{t.onboardingWelcomeDemoTranslate}</header>
            <div className="ob-tr-stack">
              <span className="ob-tr-from">{translateFrom}</span>
              <span className="ob-tr-to">{translateTo}</span>
              <span className="ob-tr-sweep" />
            </div>
          </div>
        </div>
      </section>

      <div className="ob-ticker ob-in" style={order(6)} aria-label={t.onboardingWelcomeTickerLabel}>
        <div className="ob-ticker-track">
          {[0, 1].map((copy) => (
            <ul key={copy} aria-hidden={copy === 1 ? true : undefined}>
              {ticker.map((item) => <li key={item}>{item}</li>)}
            </ul>
          ))}
        </div>
      </div>
    </div>
  )
}
