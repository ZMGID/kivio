import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { Lang } from '../components/i18n'
import type { StreamCoarse } from './streamingStore'

/** 空态只在没有历史消息或实时展示内容时出现。 */
export function isEmptyChatPresentation(messageCount: number, stream: StreamCoarse): boolean {
  // 首轮结束会先撤掉临时用户消息；冻结预览要等 React 提交正式历史后才释放。
  // 此时切回欢迎页会卸载整份消息列表，丢掉正文节点、测量缓存和滚动位置。
  return messageCount === 0 && !stream.streaming && !stream.streamFrozen && !stream.streamError
}

/** 空会话标题：短、跟墨团配。换句间隔随机，大约一分钟上下。 */
export const EMPTY_HERO_ROTATE_MIN_MS = 45_000
export const EMPTY_HERO_ROTATE_MAX_MS = 105_000
export const EMPTY_HERO_ROTATE_MS = EMPTY_HERO_ROTATE_MAX_MS

export function nextEmptyHeroRotateMs(random = Math.random): number {
  const u = random()
  const t = 1 - (1 - u) * (1 - u)
  return EMPTY_HERO_ROTATE_MIN_MS + (EMPTY_HERO_ROTATE_MAX_MS - EMPTY_HERO_ROTATE_MIN_MS) * t
}

const GREETINGS: Record<Lang, readonly string[]> = {
  zh: [
    '从一个想法开始', '今天想聊点什么', '有个问题也行', '一起理理思路',
    '慢慢说，我在', '想从哪里开始', '把想法放这儿', '小事也可以聊',
    '来，一起想想', '有灵感了吗', '准备好了就开始', '我们一步步来',
  ],
  en: [
    'Start with an idea', 'What’s on your mind?', 'A question works too', 'Let’s think it through',
    'Take your time. I’m here.', 'Where shall we start?', 'Bring your ideas here', 'Small things count too',
    'Let’s figure it out', 'Any sparks of inspiration?', 'Ready when you are', 'One step at a time',
  ],
}

export function emptyHeroGreetings(lang: Lang): readonly string[] {
  return GREETINGS[lang]
}

function greetingIndex(seed: string | null | undefined, length: number): number {
  if (!seed || length <= 0) return 0
  let h = 0
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) | 0
  return Math.abs(h) % length
}

export function emptyHeroPinnedLine(opts: {
  lang: Lang
  assistantName?: string | null
  projectName?: string | null
  setName?: string | null
}): string | null {
  const assistant = opts.assistantName?.trim()
  if (assistant) return assistant
  const project = opts.projectName?.trim()
  if (project) return opts.lang === 'zh' ? `在「${project}」` : `In “${project}”`
  const set = opts.setName?.trim()
  if (set) return opts.lang === 'zh' ? `在「${set}」` : `In “${set}”`
  return null
}

/** 空态闲置时轮换问候；助手 / 项目 / 集名钉住不转。 */
export function useEmptyHeroLine(opts: {
  lang: Lang
  assistantName?: string | null
  projectName?: string | null
  setName?: string | null
  seed?: string | null
  active?: boolean
}): string {
  const pinned = emptyHeroPinnedLine(opts)
  const greetings = GREETINGS[opts.lang]
  const startIndex = useMemo(
    () => greetingIndex(opts.seed, greetings.length),
    [opts.seed, greetings.length],
  )
  const [tick, setTick] = useState(0)

  useEffect(() => {
    setTick(0)
  }, [startIndex, pinned])

  useEffect(() => {
    if (pinned || opts.active === false) return
    let id = 0
    const arm = () => {
      id = window.setTimeout(() => {
        setTick((n) => n + 1)
        arm()
      }, nextEmptyHeroRotateMs())
    }
    arm()
    return () => window.clearTimeout(id)
  }, [pinned, opts.active, startIndex])

  if (pinned) return pinned
  return greetings[(startIndex + tick) % greetings.length]
}

export const EMPTY_HERO_JAB_MS = 5000

const JABS: Record<Lang, {
  mild: readonly string[]
  mid: readonly string[]
  hot: readonly string[]
  melt: readonly string[]
}> = {
  zh: {
    mild: ['嗯？我在', '收到一枚轻戳', '回头看你一眼', '有什么悄悄话', '你好呀', '听着呢'],
    mid: ['有点痒', '再戳就变方了', '我可要眨眼了', '你找到我的开关了', '戳戳收到啦', '好啦，看到你了'],
    hot: ['脸有点热了', '方一下，缓缓', '今天的戳戳有点多', '让我站稳一点', '快戳成果冻了', '轻一点嘛'],
    melt: ['晕乎乎的', '让我缓一缓', '先把自己揉圆', '歇一会儿再玩', '好啦，陪你聊点什么'],
  },
  en: {
    mild: ['Hm? I’m here.', 'A little poke, received.', 'Looking your way.', 'Got a secret?', 'Oh, hello.', 'I’m listening.'],
    mid: ['That tickles.', 'You’ll turn me square.', 'Time for a blink.', 'You found my button.', 'Poke received!', 'Okay, I see you.'],
    hot: ['Getting a little warm.', 'Feeling rather square.', 'That’s a lot of pokes.', 'Let me find my balance.', 'Turning into jelly.', 'Gently, please.'],
    melt: ['A little dizzy.', 'Give me a moment.', 'Rounding myself out.', 'Let’s take a tiny break.', 'So, what shall we chat about?'],
  },
}

export function emptyHeroJabPool(lang: Lang, streak: number): readonly string[] {
  const packs = JABS[lang]
  if (streak >= 9) return packs.melt
  if (streak >= 6) return packs.hot
  if (streak >= 3) return packs.mid
  return packs.mild
}

export function emptyHeroJab(
  lang: Lang,
  streak: number,
  last?: string | null,
  random = Math.random,
): string {
  const pool = emptyHeroJabPool(lang, streak)
  const choices = last ? pool.filter((line) => line !== last) : pool
  const list = choices.length > 0 ? choices : pool
  return list[Math.floor(random() * list.length)]
}

export function useEmptyHeroJab(lang: Lang) {
  const [jab, setJab] = useState<string | null>(null)
  const lastRef = useRef<string | null>(null)
  const timerRef = useRef(0)

  useEffect(() => () => window.clearTimeout(timerRef.current), [])

  useEffect(() => {
    setJab(null)
    lastRef.current = null
  }, [lang])

  const onPoke = useCallback((streak: number) => {
    const line = emptyHeroJab(lang, streak, lastRef.current)
    lastRef.current = line
    setJab(line)
    window.clearTimeout(timerRef.current)
    timerRef.current = window.setTimeout(() => {
      setJab(null)
      lastRef.current = null
    }, EMPTY_HERO_JAB_MS)
  }, [lang])

  return { jab, onPoke }
}
