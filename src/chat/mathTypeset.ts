import katex from 'katex'
import 'katex/dist/katex.min.css'

const ERROR_COLOR = 'var(--color-muted-foreground)'

/**
 * 把一条 TeX 排成 HTML。与原 rehype-katex 行为一致：先按严格模式解析，
 * 失败时（例如流式输出写到一半）退回不抛错的渲染，以错误色显示源码。
 */
export function typesetMath(tex: string, display: boolean): string | null {
  try {
    return katex.renderToString(tex, { displayMode: display, throwOnError: true })
  } catch {
    try {
      return katex.renderToString(tex, { displayMode: display, throwOnError: false, strict: 'ignore', errorColor: ERROR_COLOR })
    } catch {
      return null
    }
  }
}
