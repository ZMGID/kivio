import { useEffect, useRef, useState } from 'react'
import type { Lang } from '../components/i18n'
import type { BlobMood } from './kivioBlobSim'

/**
 * 流状态行上墨团偶尔冒的一句嘴（「1s · 72 tokens · 1 running · 琢磨中」的最后一截）。
 * 不是每时每刻都说：想 / 搜 / 干活 / 说话这四个阶段要先待够一会才开口，说完收回、隔一阵再来；
 * 翻车 / 收工 / 等你这三种是边沿事件，进去就说、一直挂着。
 */
export type QuipMood = Exclude<BlobMood, 'idle'>

export const STATUS_QUIPS: Record<Lang, Record<QuipMood, readonly string[]>> = {
  zh: {
    think: ['让我想一想', '理一理思路', '换个角度想想', '把线索串起来'],
    search: ['找找相关线索', '翻一翻资料', '看看还有什么', '顺着线索找找'],
    work: ['正在动手', '一步一步来', '专心处理这一步', '继续往下做'],
    speak: ['慢慢说清楚', '把思路写下来', '整理成容易读的样子', '接着往下说'],
    error: ['这里出了点问题', '这一步没成功', '暂时没能完成'],
    done: ['这一轮完成了', '结果在这里', '做好了，看看吧'],
    wait: ['等你一句话', '接下来听你的', '你来决定'],
  },
  en: {
    think: ['Thinking it through.', 'Sorting out the ideas.', 'Trying another angle.', 'Connecting the dots.'],
    search: ['Looking for clues.', 'Checking the sources.', 'Seeing what else is there.', 'Following a lead.'],
    work: ['Working on it.', 'One step at a time.', 'Focusing on this step.', 'Moving things along.'],
    speak: ['Putting it into words.', 'Writing out the thinking.', 'Making it easier to follow.', 'Continuing the explanation.'],
    error: ['Something went wrong here.', 'That step didn’t work.', 'Couldn’t finish this time.'],
    done: ['This round is complete.', 'Here’s the result.', 'Ready for a look.'],
    wait: ['Ready when you are.', 'Over to you.', 'Your call.'],
  },
}

/**
 * 何时开口。`first` = 进入该心情多久后到第一个「可能开口」的点；每个点按 `chance` 掷一次，
 * 没中就闭嘴等下一个点；`show` = 说了的话挂多久（null = 常驻）；`gap` = 两个点之间隔多久。
 * 整体调得偏安静：一轮几十秒的生成里大概率只冒一句，甚至一句都没有——每次都说就不是「偶尔」了。
 */
export interface QuipPlan {
  first: number
  show: number | null
  gap: [number, number]
  chance: number
}

export function quipPlan(mood: BlobMood): QuipPlan | null {
  switch (mood) {
    case 'think':
      return { first: 12000, show: 4000, gap: [18000, 40000], chance: 0.4 }
    case 'search':
      return { first: 6000, show: 3500, gap: [12000, 30000], chance: 0.35 }
    case 'work':
      return { first: 10000, show: 3500, gap: [18000, 40000], chance: 0.35 }
    case 'speak':
      return { first: 15000, show: 3000, gap: [25000, 50000], chance: 0.3 }
    case 'error':
      return { first: 500, show: null, gap: [0, 0], chance: 0.5 }
    case 'done':
      return { first: 0, show: null, gap: [0, 0], chance: 0.4 }
    case 'wait':
      return { first: 1200, show: null, gap: [15000, 15000], chance: 0.45 }
    default:
      return null
  }
}

export function pickQuip(
  lang: Lang,
  mood: BlobMood,
  last?: string | null,
  random: () => number = Math.random,
): string | null {
  if (mood === 'idle') return null
  const pool = STATUS_QUIPS[lang][mood]
  if (!pool || pool.length === 0) return null
  const choices = last ? pool.filter((line) => line !== last) : pool
  const list = choices.length > 0 ? choices : pool
  return list[Math.min(list.length - 1, Math.floor(random() * list.length))]
}

function within([a, b]: [number, number], random: () => number): number {
  return a + (b - a) * random()
}

/** 按 quipPlan 的节拍说话；心情一换就闭嘴重排。`enabled=false` 全程不吭声。 */
export function useStatusQuip(mood: BlobMood, lang: Lang, enabled = true): string | null {
  const [quip, setQuip] = useState<string | null>(null)
  const lastRef = useRef<string | null>(null)

  useEffect(() => {
    setQuip(null)
    lastRef.current = null
    const plan = quipPlan(mood)
    if (!plan || !enabled) return
    let id = 0
    const say = () => {
      const speak = Math.random() < plan.chance
      const line = speak ? pickQuip(lang, mood, lastRef.current) : null
      if (line) lastRef.current = line
      setQuip(line)
      const next = () => {
        if (plan.gap[1] > 0) id = window.setTimeout(say, within(plan.gap, Math.random))
      }
      if (plan.show == null || !line) {
        next()
        return
      }
      id = window.setTimeout(() => {
        setQuip(null)
        next()
      }, plan.show)
    }
    id = window.setTimeout(say, plan.first)
    return () => window.clearTimeout(id)
  }, [mood, lang, enabled])

  return quip
}
