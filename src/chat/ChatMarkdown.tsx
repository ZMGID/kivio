import type { Components } from 'react-markdown'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import { normalizeMarkdownForRender } from './markdownUtils'
import { MarkdownErrorBoundary } from './MarkdownErrorBoundary'

interface ChatMarkdownProps {
  content: string
}

const proseClass =
  'prose prose-sm dark:prose-invert max-w-none break-words text-[15px] leading-[1.7] text-neutral-900 dark:text-neutral-100 prose-p:my-2 prose-headings:my-3 prose-ul:my-2 prose-ol:my-2 prose-pre:my-2 prose-li:my-0.5 prose-table:my-3 prose-table:shadow-none'

const markdownComponents: Components = {
  table: ({ children }) => (
    <div className="my-3 max-w-full overflow-x-auto">
      <table className="w-full min-w-[240px] border-collapse text-[13px] leading-snug">
        {children}
      </table>
    </div>
  ),
  thead: ({ children }) => (
    <thead className="bg-neutral-50 dark:bg-neutral-800/90">{children}</thead>
  ),
  th: ({ children }) => (
    <th className="border border-neutral-200/90 px-3 py-2 text-left font-semibold text-neutral-800 dark:border-neutral-700 dark:text-neutral-100">
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td className="border border-neutral-200/90 px-3 py-2 align-top text-neutral-700 dark:border-neutral-700 dark:text-neutral-300">
      {children}
    </td>
  ),
}

export function ChatMarkdown({ content }: ChatMarkdownProps) {
  const normalized = normalizeMarkdownForRender(content)
  return (
    <div className={proseClass}>
      <MarkdownErrorBoundary fallbackText={content}>
        <ReactMarkdown
          remarkPlugins={[remarkGfm, remarkMath]}
          rehypePlugins={[rehypeKatex]}
          components={markdownComponents}
        >
          {normalized}
        </ReactMarkdown>
      </MarkdownErrorBoundary>
    </div>
  )
}
