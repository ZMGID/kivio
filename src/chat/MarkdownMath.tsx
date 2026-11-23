import { memo, useEffect, useMemo, useSyncExternalStore } from 'react'
import { loadMarkdownMath, readMarkdownMathTypeset, subscribeMarkdownMath } from './markdownMathPlugin'

/**
 * 一条公式。KaTeX 未加载时显示 TeX 源码并触发加载；加载过一次后同步排版，
 * 虚拟列表重新挂载或切换对话时不会再闪源码。排版失败同样保留源码。
 */
export const MarkdownMath = memo(function MarkdownMath({ tex, display }: { tex: string; display: boolean }) {
  const typeset = useSyncExternalStore(subscribeMarkdownMath, readMarkdownMathTypeset, readMarkdownMathTypeset)
  useEffect(() => {
    if (typeset) return
    loadMarkdownMath().catch((error: unknown) => console.error('[markdown] KaTeX failed to load', error))
  }, [typeset])
  const html = useMemo(() => typeset?.(tex, display) ?? null, [typeset, tex, display])
  const Tag = display ? 'div' : 'span'
  const kind = display ? 'display' : 'inline'
  if (html == null) {
    return (
      <Tag data-markdown-math={kind} className={display ? 'my-3 whitespace-pre-wrap text-[var(--text-muted)]' : 'text-[var(--text-muted)]'}>
        {display ? `$$\n${tex}\n$$` : `$${tex}$`}
      </Tag>
    )
  }
  return <Tag data-markdown-math={kind} dangerouslySetInnerHTML={{ __html: html }} />
})
