import { isValidElement, memo, useMemo, useState } from 'react'
import { Code2, ExternalLink, Eye } from 'lucide-react'
import type { Components, UrlTransform } from 'react-markdown'
import ReactMarkdown, { defaultUrlTransform } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import { normalizeMarkdownForRender } from './markdownUtils'
import { MarkdownErrorBoundary } from './MarkdownErrorBoundary'
import type { ChatToolArtifact } from './types'
import { api } from '../api/tauri'

interface ChatMarkdownProps {
  content: string
  artifacts?: ChatToolArtifact[]
  onImageClick?: (src: string, alt: string, name?: string) => void
}

const proseClass =
  'prose prose-sm dark:prose-invert max-w-none break-words text-[15px] leading-[1.7] text-neutral-900 dark:text-neutral-100 prose-p:my-2 prose-headings:my-3 prose-ul:my-2 prose-ol:my-2 prose-pre:my-2 prose-li:my-0.5 prose-table:my-3 prose-table:shadow-none'

function codeChildrenToString(children: unknown): string {
  if (Array.isArray(children)) return children.map((child) => String(child ?? '')).join('')
  return typeof children === 'string' ? children : String(children ?? '')
}

function htmlPreviewSrcDoc(html: string): string {
  const trimmed = html.trim()
  if (!trimmed) return html

  if (/^(?:<!doctype\s+html[^>]*>\s*)?<html[\s>]/i.test(trimmed)) {
    let repaired = trimmed
    if (/<style[\s>]/i.test(repaired) && !/<\/style>/i.test(repaired)) {
      repaired += '\n</style>'
    }
    if (/<head[\s>]/i.test(repaired) && !/<\/head>/i.test(repaired)) {
      repaired += '\n</head>'
    }
    if (!/<body[\s>]/i.test(repaired)) {
      repaired += '\n<body></body>'
    }
    if (!/<\/body>/i.test(repaired)) {
      repaired += '\n</body>'
    }
    if (!/<\/html>/i.test(repaired)) {
      repaired += '\n</html>'
    }
    return repaired
  }

  return html
}

function HtmlCodePreview({ html }: { html: string }) {
  const [view, setView] = useState<'preview' | 'source'>('preview')
  const previewHtml = useMemo(() => htmlPreviewSrcDoc(html), [html])

  const openInBrowser = () => {
    void api.openHtmlPreview(previewHtml).catch((err) => {
      console.error('Failed to open HTML preview:', err)
    })
  }

  return (
    <>
      <div className="my-3 overflow-hidden rounded-lg border border-neutral-200 bg-white dark:border-neutral-700 dark:bg-neutral-950">
        {view === 'preview' ? (
          <iframe
            title="HTML 预览"
            srcDoc={previewHtml}
            className="h-[520px] w-full border-0 bg-white"
          />
        ) : (
          <pre className="m-0 max-h-[520px] overflow-auto bg-neutral-950 p-4 text-[12px] leading-relaxed text-neutral-100">
            <code>{html}</code>
          </pre>
        )}
      </div>
      <div className="-mt-1 mb-2 flex justify-end gap-0.5">
        <button
          type="button"
          onClick={() => setView((current) => (current === 'preview' ? 'source' : 'preview'))}
          className="rounded p-1 text-neutral-400 transition-colors hover:bg-neutral-100 hover:text-neutral-600 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
          title={view === 'preview' ? '查看源码' : '查看预览'}
          aria-label={view === 'preview' ? '查看源码' : '查看预览'}
        >
          {view === 'preview' ? <Code2 size={14} strokeWidth={2} /> : <Eye size={14} strokeWidth={2} />}
        </button>
        <button
          type="button"
          onClick={openInBrowser}
          className="rounded p-1 text-neutral-400 transition-colors hover:bg-neutral-100 hover:text-neutral-600 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
          title="在浏览器打开"
          aria-label="在浏览器打开"
        >
          <ExternalLink size={14} strokeWidth={2} />
        </button>
      </div>
    </>
  )
}

const markdownComponents: Components = {
  pre: ({ children }) => {
    const child = Array.isArray(children) ? children[0] : children
    if (isValidElement<{ className?: string; children?: unknown }>(child)) {
      const languageMatch = /language-([\w-]+)/.exec(child.props.className ?? '')
      if (languageMatch?.[1]?.toLowerCase() === 'html') {
        return <HtmlCodePreview html={codeChildrenToString(child.props.children)} />
      }
    }
    return <pre>{children}</pre>
  },
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

function artifactDataUrl(artifact: ChatToolArtifact): string {
  return artifact.dataUrl ?? artifact.data_url ?? ''
}

function safeDecodeURIComponent(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

function artifactKey(name: string): string {
  return safeDecodeURIComponent(name)
    .trim()
    .replace(/^\.?\//, '')
    .replace(/\\/g, '/')
    .toLowerCase()
}

function artifactBasename(name: string): string {
  return artifactKey(name).split('/').filter(Boolean).pop() ?? artifactKey(name)
}

function isExternalOrAbsoluteImageSrc(src: string): boolean {
  return /^(https?:|data:|blob:|tauri:|asset:|file:|\/)/i.test(src)
}

function isSafeImageDataUrl(src: string): boolean {
  return /^data:image\/(?:png|jpe?g|gif|webp|svg\+xml);base64,[a-z0-9+/=\s]+$/i.test(src.trim())
}

const chatMarkdownUrlTransform: UrlTransform = (url, key, node) => {
  if (key === 'src' && node.tagName === 'img' && isSafeImageDataUrl(url)) {
    return url
  }
  return defaultUrlTransform(url)
}

function buildArtifactLookup(artifacts: ChatToolArtifact[]): Map<string, string> {
  const lookup = new Map<string, string>()
  for (const artifact of artifacts) {
    const dataUrl = artifactDataUrl(artifact)
    if (!artifact.name || !dataUrl.startsWith('data:image/')) continue
    lookup.set(artifactKey(artifact.name), dataUrl)
    lookup.set(artifactBasename(artifact.name), dataUrl)
  }
  return lookup
}

function ChatMarkdownComponent({ content, artifacts = [], onImageClick }: ChatMarkdownProps) {
  const normalized = useMemo(() => normalizeMarkdownForRender(content), [content])
  const components = useMemo<Components>(() => {
    const artifactLookup = buildArtifactLookup(artifacts)
    return {
      ...markdownComponents,
      img: ({ src, alt }) => {
        const rawSrc = typeof src === 'string' ? src : ''
        const resolvedSrc = rawSrc && !isExternalOrAbsoluteImageSrc(rawSrc)
          ? artifactLookup.get(artifactKey(rawSrc)) ?? artifactLookup.get(artifactBasename(rawSrc)) ?? rawSrc
          : rawSrc
        const altText = alt ?? ''
        return (
          <button
            type="button"
            className="my-3 block max-w-full cursor-zoom-in rounded-md p-0 text-left"
            onClick={() => {
              if (resolvedSrc) onImageClick?.(resolvedSrc, altText, rawSrc)
            }}
            aria-label="预览图片"
          >
            <img
              src={resolvedSrc}
              alt={altText}
              loading="lazy"
              className="max-h-[420px] max-w-full rounded-md border border-neutral-200/90 bg-white object-contain dark:border-neutral-700 dark:bg-neutral-900"
            />
          </button>
        )
      },
    }
  }, [artifacts, onImageClick])

  return (
    <div className={proseClass}>
      <MarkdownErrorBoundary fallbackText={content}>
        <ReactMarkdown
          remarkPlugins={[remarkGfm, remarkMath]}
          rehypePlugins={[rehypeKatex]}
          components={components}
          urlTransform={chatMarkdownUrlTransform}
        >
          {normalized}
        </ReactMarkdown>
      </MarkdownErrorBoundary>
    </div>
  )
}

// memo：仅当 content / artifacts 变化时才重渲染（配合 MessageBubble 的 memo）
export const ChatMarkdown = memo(ChatMarkdownComponent)
