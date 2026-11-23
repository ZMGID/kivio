import type { Element, ElementContent, Root, RootContent } from 'hast'
import remarkMath from 'remark-math'
import type { MathPlugin } from 'streamdown'
import { scheduleIdleTask } from './idleTask'

/** remark-math 产出的公式节点在 hast 中改写为此元素，由 MarkdownMath 组件排版。 */
export const MARKDOWN_MATH_TAG = 'kivio-math'

type Typeset = (tex: string, display: boolean) => string | null

let typeset: Typeset | null = null
let loading: Promise<Typeset> | null = null
let preloadScheduled = false
const listeners = new Set<() => void>()

/**
 * 加载 KaTeX 排版模块（约 600 KB，独立分包）。同一窗口只加载一次；
 * 加载失败后清空记录，下一次挂载公式时重试。
 */
export function loadMarkdownMath(): Promise<Typeset> {
  loading ??= import('./mathTypeset').then((module) => {
    typeset = module.typesetMath
    for (const listener of listeners) listener()
    return module.typesetMath
  }, (error: unknown) => {
    loading = null
    throw error
  })
  return loading
}

/** 空闲时预取 KaTeX，让打开含公式的对话时直接显示排版结果，而不是先闪源码。 */
export function preloadMarkdownMathWhenIdle(): void {
  if (typeset || preloadScheduled) return
  preloadScheduled = true
  scheduleIdleTask(() => {
    loadMarkdownMath().catch(() => { preloadScheduled = false })
  })
}

export function subscribeMarkdownMath(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function readMarkdownMathTypeset(): Typeset | null {
  return typeset
}

function classNames(element: Element): string[] {
  const value = element.properties?.className
  return Array.isArray(value) ? value.map(String) : typeof value === 'string' ? value.split(/\s+/) : []
}

function textContent(node: ElementContent): string {
  if (node.type === 'text') return node.value
  if (node.type === 'element') return node.children.map(textContent).join('')
  return ''
}

function mathNode(element: Element): { tex: string; display: boolean } | null {
  if (element.tagName === 'code') {
    const classes = classNames(element)
    if (classes.includes('math-display')) return { tex: element.children.map(textContent).join(''), display: true }
    // The sanitizer can leave language-math as the only class on inline formulas.
    if (classes.includes('math-inline') || classes.includes('language-math')) return { tex: element.children.map(textContent).join(''), display: false }
    return null
  }
  if (element.tagName === 'pre') {
    const code = element.children.find((child): child is Element => child.type === 'element' && child.tagName === 'code')
    if (code && classNames(code).some(name => name === 'math-display' || name === 'language-math')) return { tex: code.children.map(textContent).join(''), display: true }
  }
  return null
}

function replaceMath<T extends RootContent | ElementContent>(children: T[]): T[] {
  return children.map((child) => {
    if (child.type !== 'element') return child
    const math = mathNode(child)
    if (!math) {
      child.children = replaceMath(child.children)
      return child
    }
    const replacement: Element = {
      type: 'element',
      tagName: MARKDOWN_MATH_TAG,
      properties: { dataMath: math.display ? 'display' : 'inline' },
      children: [{ type: 'text', value: math.tex }],
      position: child.position,
    }
    return replacement as T
  })
}

/** 只做节点改写，不引入 KaTeX；在 streamdown 的 HTML 清洗之后运行。 */
export function rehypeMarkdownMath() {
  return (tree: Root) => {
    tree.children = replaceMath(tree.children)
  }
}

/** streamdown 数学插件：解析交给 remark-math，排版交给按需加载的 MarkdownMath。 */
export const markdownMathPlugin: MathPlugin = {
  name: 'katex',
  type: 'math',
  remarkPlugin: [remarkMath, { singleDollarTextMath: true }],
  rehypePlugin: rehypeMarkdownMath,
}
