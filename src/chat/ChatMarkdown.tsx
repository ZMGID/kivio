import { isValidElement, memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Check, Code2, Copy, ExternalLink, Eye, Loader2 } from 'lucide-react'
import type { Components, UrlTransform } from 'react-markdown'
import ReactMarkdown, { defaultUrlTransform } from 'react-markdown'
import type { PluggableList } from 'unified'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'
import katex from 'katex'
import katexCss from 'katex/dist/katex.min.css?inline'
import { normalizeMarkdownForRender } from './markdownUtils'
import { MarkdownErrorBoundary } from './MarkdownErrorBoundary'
import type { ChatToolArtifact } from './types'
import type { KbHitView } from './knowledgeBaseHits'
import { remarkCitations } from './citations'
import { api } from '../api/tauri'
import { copyToClipboard } from '../utils/clipboard'

interface ChatMarkdownProps {
  content: string
  artifacts?: ChatToolArtifact[]
  onImageClick?: (src: string, alt: string, name?: string) => void
  variant?: 'default' | 'reasoning'
  /** 知识库引用：把答案里的 `[n]` 渲染成可点来源片段（n → 命中片段）。 */
  citations?: Map<number, KbHitView>
}

const proseClass =
  'chat-markdown prose prose-sm dark:prose-invert max-w-none break-words text-[15px] leading-[1.7] text-neutral-900 dark:text-neutral-100 prose-p:my-2 prose-headings:my-3 prose-ul:my-2 prose-ol:my-2 prose-pre:my-2 prose-li:my-0.5 prose-table:my-3 prose-table:shadow-none prose-code:rounded prose-code:bg-neutral-100 prose-code:px-1 prose-code:py-0.5 prose-code:font-medium prose-code:text-neutral-800 prose-code:before:content-none prose-code:after:content-none dark:prose-code:bg-neutral-800 dark:prose-code:text-neutral-100'

const reasoningProseClass =
  'chat-markdown chat-reasoning-markdown prose prose-sm dark:prose-invert max-w-none break-words text-sm leading-relaxed text-neutral-400 dark:text-neutral-500 prose-p:my-1 prose-p:first:mt-0 prose-p:last:mb-0 prose-headings:my-2 prose-ul:my-1 prose-ol:my-1 prose-pre:my-2 prose-li:my-0.5 prose-table:my-2 prose-table:shadow-none prose-code:rounded prose-code:bg-neutral-100 prose-code:px-1 prose-code:py-0.5 prose-code:font-medium prose-code:text-neutral-500 prose-code:before:content-none prose-code:after:content-none dark:prose-code:bg-neutral-800 dark:prose-code:text-neutral-400'

function codeChildrenToString(children: unknown): string {
  if (Array.isArray(children)) return children.map((child) => String(child ?? '')).join('')
  return typeof children === 'string' ? children : String(children ?? '')
}

type HighlightToken = {
  text: string
  className?: string
}

type TokenRule = {
  className: string
  pattern: RegExp
}

const LANGUAGE_LABELS: Record<string, string> = {
  bash: 'Shell',
  cjs: 'JavaScript',
  css: 'CSS',
  html: 'HTML',
  js: 'JavaScript',
  javascript: 'JavaScript',
  json: 'JSON',
  jsx: 'JavaScript',
  markdown: 'Markdown',
  md: 'Markdown',
  mermaid: 'Mermaid',
  py: 'Python',
  python: 'Python',
  rs: 'Rust',
  rust: 'Rust',
  sh: 'Shell',
  shell: 'Shell',
  ts: 'TypeScript',
  tsx: 'TypeScript',
  typescript: 'TypeScript',
  xml: 'XML',
  yaml: 'YAML',
  yml: 'YAML',
}

const jsKeywords =
  'abstract|as|async|await|break|case|catch|class|const|continue|debugger|declare|default|delete|do|else|enum|export|extends|finally|for|from|function|get|if|implements|import|in|infer|instanceof|interface|keyof|let|module|namespace|new|of|private|protected|public|readonly|return|satisfies|set|static|super|switch|throw|try|type|typeof|var|void|while|with|yield'
const rustKeywords =
  'as|async|await|break|const|continue|crate|dyn|else|enum|extern|false|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|true|type|unsafe|use|where|while'
const pythonKeywords =
  'and|as|assert|async|await|break|class|continue|def|del|elif|else|except|False|finally|for|from|global|if|import|in|is|lambda|None|nonlocal|not|or|pass|raise|return|True|try|while|with|yield'

function normalizeCodeLanguage(language?: string): string {
  return (language ?? '').trim().toLowerCase().replace(/^language-/, '')
}

function codeLanguageLabel(language: string): string {
  if (!language) return 'Code'
  return LANGUAGE_LABELS[language] ?? language.toUpperCase()
}

function tokenPattern(source: string): RegExp {
  return new RegExp(source, 'y')
}

function scanTokens(code: string, rules: TokenRule[]): HighlightToken[] {
  const tokens: HighlightToken[] = []
  let index = 0

  while (index < code.length) {
    let matched = false

    for (const rule of rules) {
      rule.pattern.lastIndex = index
      const match = rule.pattern.exec(code)
      if (!match?.[0]) continue
      tokens.push({ text: match[0], className: rule.className })
      index += match[0].length
      matched = true
      break
    }

    if (!matched) {
      const previous = tokens[tokens.length - 1]
      if (previous && !previous.className) {
        previous.text += code[index]
      } else {
        tokens.push({ text: code[index] })
      }
      index += 1
    }
  }

  return tokens
}

function cLikeRules(keywordSource: string): TokenRule[] {
  return [
    { className: 'text-neutral-400', pattern: tokenPattern(String.raw`\/\/[^\n]*|\/\*[\s\S]*?\*\/`) },
    { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
    { className: 'text-blue-700', pattern: tokenPattern(String.raw`\b(?:${keywordSource})\b`) },
    { className: 'text-amber-700', pattern: tokenPattern(String.raw`\b(?:true|false|null|undefined|Some|None|Ok|Err)\b`) },
    { className: 'text-cyan-700', pattern: tokenPattern(String.raw`\b[A-Za-z_$][\w$]*(?=\s*\()`) },
    { className: 'text-violet-700', pattern: tokenPattern(String.raw`\b[A-Z][A-Za-z0-9_$]*\b`) },
    { className: 'text-orange-700', pattern: tokenPattern(String.raw`\b(?:0x[\da-fA-F]+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b`) },
    { className: 'text-neutral-500', pattern: tokenPattern(String.raw`=>|->|::|[{}()[\].,;:+\-*/%=&|!<>?]+`) },
  ]
}

function jsxRules(keywordSource: string): TokenRule[] {
  return [
    { className: 'text-neutral-400', pattern: tokenPattern(String.raw`\/\/[^\n]*|\/\*[\s\S]*?\*\/`) },
    { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
    { className: 'text-blue-700', pattern: tokenPattern(String.raw`<\/?[A-Za-z][\w:.-]*`) },
    { className: 'text-amber-700', pattern: tokenPattern(String.raw`\b[A-Za-z_:][\w:.-]*(?=\s*=)`) },
    { className: 'text-blue-700', pattern: tokenPattern(String.raw`\b(?:${keywordSource})\b`) },
    { className: 'text-amber-700', pattern: tokenPattern(String.raw`\b(?:true|false|null|undefined)\b`) },
    { className: 'text-cyan-700', pattern: tokenPattern(String.raw`\b[A-Za-z_$][\w$]*(?=\s*\()`) },
    { className: 'text-violet-700', pattern: tokenPattern(String.raw`\b[A-Z][A-Za-z0-9_$]*\b`) },
    { className: 'text-orange-700', pattern: tokenPattern(String.raw`\b(?:0x[\da-fA-F]+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b`) },
    { className: 'text-neutral-500', pattern: tokenPattern(String.raw`\/?>|=>|[{}()[\].,;:+\-*/%=&|!<>?]+`) },
  ]
}

function looksLikeJsx(code: string): boolean {
  return /<\/?[A-Za-z][\w:.-]*(?:\s|>|\/>)/.test(code)
}

function rulesForLanguage(language: string, code = ''): TokenRule[] {
  if (language === 'css') {
    return [
      { className: 'text-neutral-400', pattern: tokenPattern(String.raw`\/\*[\s\S]*?\*\/`) },
      { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
      { className: 'text-rose-700', pattern: tokenPattern(String.raw`[#.][A-Za-z_][\w-]*`) },
      { className: 'text-cyan-700', pattern: tokenPattern(String.raw`@[A-Za-z-]+`) },
      { className: 'text-blue-700', pattern: tokenPattern(String.raw`\b[A-Za-z-]+(?=\s*:)`) },
      { className: 'text-orange-700', pattern: tokenPattern(String.raw`#[\da-fA-F]{3,8}\b|\b\d+(?:\.\d+)?(?:px|rem|em|%|vh|vw|s|ms)?\b`) },
      { className: 'text-violet-700', pattern: tokenPattern(String.raw`\b(?:border-box|flex|grid|block|inline|none|relative|absolute|fixed|sticky|solid|transparent)\b`) },
      { className: 'text-neutral-500', pattern: tokenPattern(String.raw`[{}():;,>+~*-]+`) },
    ]
  }

  if (language === 'html' || language === 'xml') {
    return [
      { className: 'text-neutral-400', pattern: tokenPattern(String.raw`<!--[\s\S]*?-->`) },
      { className: 'text-blue-700', pattern: tokenPattern(String.raw`<\/?[A-Za-z][\w:-]*`) },
      { className: 'text-amber-700', pattern: tokenPattern(String.raw`\b[A-Za-z_:][\w:.-]*(?=\=)`) },
      { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
      { className: 'text-neutral-500', pattern: tokenPattern(String.raw`\/?>|=`) },
    ]
  }

  if (language === 'json') {
    return [
      { className: 'text-blue-700', pattern: tokenPattern(String.raw`"(?:\\.|[^"\\])*"(?=\s*:)`) },
      { className: 'text-emerald-700', pattern: tokenPattern(String.raw`"(?:\\.|[^"\\])*"`) },
      { className: 'text-amber-700', pattern: tokenPattern(String.raw`\b(?:true|false|null)\b`) },
      { className: 'text-orange-700', pattern: tokenPattern(String.raw`-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b`) },
      { className: 'text-neutral-500', pattern: tokenPattern(String.raw`[{}[\]:,]+`) },
    ]
  }

  if (language === 'py' || language === 'python') {
    return [
      { className: 'text-neutral-400', pattern: tokenPattern(String.raw`#[^\n]*`) },
      { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'''[\s\S]*?'''|"""[\s\S]*?"""|'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
      { className: 'text-blue-700', pattern: tokenPattern(String.raw`\b(?:${pythonKeywords})\b`) },
      { className: 'text-cyan-700', pattern: tokenPattern(String.raw`\b[A-Za-z_]\w*(?=\s*\()`) },
      { className: 'text-orange-700', pattern: tokenPattern(String.raw`\b\d+(?:\.\d+)?\b`) },
      { className: 'text-neutral-500', pattern: tokenPattern(String.raw`[{}()[\].,;:+\-*/%=&|!<>?]+`) },
    ]
  }

  if (language === 'sh' || language === 'shell' || language === 'bash') {
    return [
      { className: 'text-neutral-400', pattern: tokenPattern(String.raw`#[^\n]*`) },
      { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
      { className: 'text-blue-700', pattern: tokenPattern(String.raw`\b(?:case|cat|cd|cp|do|done|echo|elif|else|esac|export|fi|for|function|git|grep|if|mkdir|mv|npm|rg|rm|sed|then|while)\b`) },
      { className: 'text-violet-700', pattern: tokenPattern(String.raw`\$[A-Za-z_]\w*|\$\{[^}]+\}`) },
      { className: 'text-orange-700', pattern: tokenPattern(String.raw`\b\d+\b`) },
      { className: 'text-neutral-500', pattern: tokenPattern(String.raw`[|&;<>(){}[\]!*?=]+`) },
    ]
  }

  if (language === 'rust' || language === 'rs') {
    return cLikeRules(rustKeywords)
  }

  if (language === 'jsx' || language === 'tsx') {
    return jsxRules(jsKeywords)
  }

  if (language === 'js' || language === 'javascript' || language === 'ts' || language === 'typescript') {
    if (looksLikeJsx(code)) return jsxRules(jsKeywords)
    return cLikeRules(jsKeywords)
  }

  return [
    { className: 'text-neutral-400', pattern: tokenPattern(String.raw`\/\/[^\n]*|#[^\n]*|\/\*[\s\S]*?\*\/`) },
    { className: 'text-emerald-700', pattern: tokenPattern(String.raw`'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"`) },
    { className: 'text-orange-700', pattern: tokenPattern(String.raw`\b\d+(?:\.\d+)?\b`) },
  ]
}

function highlightCode(code: string, language: string) {
  return scanTokens(code, rulesForLanguage(language, code)).map((token, index) => (
    token.className
      ? <span key={index} className={token.className}>{token.text}</span>
      : token.text
  ))
}

function normalizeCodeBlockText(code: string): string {
  return code.replace(/\n$/, '')
}

function CodeBlock({ code, language }: { code: string; language: string }) {
  const normalizedCode = useMemo(() => normalizeCodeBlockText(code), [code])
  const highlighted = useMemo(
    () => highlightCode(normalizedCode, language),
    [normalizedCode, language],
  )
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    const ok = await copyToClipboard(normalizedCode)
    if (!ok) return
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1600)
  }

  return (
    <figure className="not-prose my-3 overflow-hidden rounded-lg border border-neutral-200/80 bg-neutral-50 text-neutral-950 shadow-sm dark:border-neutral-300/80 dark:bg-neutral-100 dark:text-neutral-950">
      <div className="flex items-center gap-2 px-4 pb-1 pt-3">
        <Code2 size={15} strokeWidth={2.4} className="shrink-0 text-neutral-800" />
        <figcaption className="text-[13px] font-semibold leading-5 text-neutral-950">
          {codeLanguageLabel(language)}
        </figcaption>
        <button
          type="button"
          onClick={() => void handleCopy()}
          className="-mr-1 ml-auto rounded-md p-1.5 text-neutral-500 transition-colors hover:bg-neutral-200/70 hover:text-neutral-900"
          title={copied ? '已复制' : '复制代码'}
          aria-label={copied ? '已复制' : '复制代码'}
        >
          {copied ? <Check size={17} strokeWidth={2.2} className="chat-motion-pop" /> : <Copy size={17} strokeWidth={2.2} />}
        </button>
      </div>
      <pre className="m-0 max-w-full overflow-x-auto bg-transparent px-4 pb-4 pt-2 text-[13px] leading-6 text-neutral-900">
        <code className="font-mono">{highlighted}</code>
      </pre>
    </figure>
  )
}

let mermaidRenderCounter = 0

function MermaidBlock({ code }: { code: string }) {
  const normalizedCode = useMemo(() => normalizeCodeBlockText(code), [code])
  const renderBaseId = useRef('')
  const renderSeq = useRef(0)
  const [view, setView] = useState<'diagram' | 'source'>('diagram')
  const [svg, setSvg] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)

  if (!renderBaseId.current) {
    mermaidRenderCounter += 1
    renderBaseId.current = `chat-mermaid-${mermaidRenderCounter}`
  }

  useEffect(() => {
    let cancelled = false
    renderSeq.current += 1
    const renderId = `${renderBaseId.current}-${renderSeq.current}`

    setLoading(true)
    setError('')
    setSvg('')

    const render = async () => {
      try {
        const { default: mermaid } = await import('mermaid')
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: 'base',
          themeVariables: {
            background: 'transparent',
            primaryColor: '#f8fafc',
            primaryBorderColor: '#94a3b8',
            primaryTextColor: '#111827',
            lineColor: '#64748b',
            secondaryColor: '#f1f5f9',
            tertiaryColor: '#ffffff',
            fontFamily: 'ui-sans-serif, system-ui, sans-serif',
          },
        })
        const result = await mermaid.render(renderId, normalizedCode)
        if (cancelled) return
        setSvg(result.svg)
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        if (!cancelled) setLoading(false)
      }
    }

    void render()
    return () => {
      cancelled = true
    }
  }, [normalizedCode])

  return (
    <figure className="not-prose my-3 overflow-hidden rounded-lg border border-neutral-200/80 bg-white text-neutral-950 shadow-sm dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-100">
      <div className="flex items-center gap-2 border-b border-neutral-200/70 px-4 py-2.5 dark:border-neutral-800">
        <Code2 size={15} strokeWidth={2.4} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
        <figcaption className="text-[13px] font-semibold leading-5">
          Mermaid
        </figcaption>
        {!error && (
          <button
            type="button"
            onClick={() => setView((current) => (current === 'diagram' ? 'source' : 'diagram'))}
            className="-mr-1 ml-auto rounded-md p-1.5 text-neutral-400 transition-colors hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
            title={view === 'diagram' ? '查看源码' : '查看图表'}
            aria-label={view === 'diagram' ? '查看源码' : '查看图表'}
          >
            {view === 'diagram' ? <Code2 size={15} strokeWidth={2} /> : <Eye size={15} strokeWidth={2} />}
          </button>
        )}
      </div>
      {view === 'source' ? (
        <CodeBlock code={normalizedCode} language="mermaid" />
      ) : loading ? (
        <div className="flex min-h-28 items-center justify-center gap-2 px-4 py-8 text-[13px] text-neutral-400 dark:text-neutral-500">
          <Loader2 size={15} className="animate-spin" />
          正在渲染图表
        </div>
      ) : error ? (
        <>
          <div className="border-b border-red-100 bg-red-50 px-4 py-2 text-[12px] leading-5 text-red-600 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300">
            Mermaid 渲染失败：{error}
          </div>
          <CodeBlock code={normalizedCode} language="mermaid" />
        </>
      ) : (
        <div
          className="max-w-full overflow-auto bg-white px-4 py-4 dark:bg-neutral-950 [&>svg]:mx-auto [&>svg]:max-w-none"
          dangerouslySetInnerHTML={{ __html: svg }}
        />
      )}
    </figure>
  )
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
      {view === 'preview' ? (
        <div className="my-3 overflow-hidden rounded-lg border border-neutral-200 bg-white dark:border-neutral-700 dark:bg-neutral-950">
          <iframe
            title="HTML 预览"
            srcDoc={previewHtml}
            className="h-[520px] w-full border-0 bg-white"
          />
        </div>
      ) : null}
      {view === 'source' ? <CodeBlock code={html} language="html" /> : null}
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
      const language = normalizeCodeLanguage(languageMatch?.[1])
      const code = codeChildrenToString(child.props.children)
      if (language === 'html') {
        return <HtmlCodePreview html={code} />
      }
      if (language === 'mermaid') {
        return <MermaidBlock code={code} />
      }
      return <CodeBlock code={code} language={language} />
    }
    return <CodeBlock code={codeChildrenToString(children)} language="" />
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
  a: ({ href, children }) => <LinkAnchor href={typeof href === 'string' ? href : ''}>{children}</LinkAnchor>,
}

function LinkAnchor({ href, children }: { href: string; children?: ReactNode }) {
  const isWeb = /^https?:\/\//i.test(href)
  return (
    <a
      href={href || undefined}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(event) => {
        // A plain <a> click would navigate the Tauri webview itself and
        // blow away the chat UI. Open web links in the system browser.
        if (!isWeb) return
        event.preventDefault()
        void api.openExternal(href).catch((err) => console.error('openExternal failed', err))
      }}
    >
      {children}
    </a>
  )
}

/** 知识库引用角标 `[n]`：点击弹出对应来源片段（文档名 · 标题 · 正文）。 */
function CitationChip({ n, hit }: { n: number; hit?: KbHitView }) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    if (!open) return
    const onDown = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [open])
  return (
    <span ref={ref} className="relative inline-block align-baseline">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="mx-0.5 rounded bg-indigo-500/15 px-1 align-baseline text-[0.82em] font-medium text-indigo-500 transition hover:bg-indigo-500/25"
        aria-label={`来源 ${n}`}
      >
        [{n}]
      </button>
      {open && (
        <span className="absolute left-0 top-full z-30 mt-1 block w-80 max-w-[80vw] rounded-lg border border-black/[0.08] bg-white p-2.5 text-left text-xs shadow-lg dark:border-white/[0.12] dark:bg-neutral-900">
          {hit ? (
            <>
              <span className="mb-1 flex items-center gap-1 font-medium text-neutral-700 dark:text-neutral-200">
                <span className="shrink-0 rounded bg-indigo-500/15 px-1 text-indigo-500">[{n}]</span>
                <span className="truncate">
                  {hit.docName}
                  {hit.headingPath ? ` · ${hit.headingPath}` : ''}
                </span>
              </span>
              <span className="block max-h-48 overflow-auto whitespace-pre-wrap break-words leading-relaxed text-neutral-600 dark:text-neutral-300">
                {hit.text}
              </span>
            </>
          ) : (
            <span className="text-neutral-400">未找到对应来源片段</span>
          )}
        </span>
      )}
    </span>
  )
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

const katexShadowCss = `${katexCss}
:host {
  display: inline-block;
  max-width: 100%;
  overflow-x: auto;
  overflow-y: hidden;
  vertical-align: middle;
  padding: 1px 2px;
  margin-top: -2px;
}
:host(.katex-lazy--display) {
  display: block;
  padding: 0;
  margin-top: 0;
  vertical-align: baseline;
}
:host(.katex-lazy--display) .katex-display {
  max-width: 100%;
  overflow: visible;
}
:host(.katex-lazy--display) .katex-display > .katex {
  display: block;
  overflow: visible;
}
`

let katexShadowSheet: CSSStyleSheet | null = null
let katexShadowSheetUnavailable = false

function getKatexShadowSheet(): CSSStyleSheet | null {
  if (katexShadowSheetUnavailable || typeof CSSStyleSheet === 'undefined') return null
  if (katexShadowSheet) return katexShadowSheet
  try {
    const sheet = new CSSStyleSheet()
    sheet.replaceSync(katexShadowCss)
    katexShadowSheet = sheet
    return sheet
  } catch {
    katexShadowSheetUnavailable = true
    return null
  }
}

function installKatexShadowStyles(root: ShadowRoot) {
  const sheet = getKatexShadowSheet()
  if (sheet && 'adoptedStyleSheets' in root) {
    try {
      if (!root.adoptedStyleSheets.includes(sheet)) {
        root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheet]
      }
      return
    } catch {
      katexShadowSheetUnavailable = true
    }
  }

  if (root.querySelector('style[data-katex-shadow-style="true"]')) return
  const style = document.createElement('style')
  style.dataset.katexShadowStyle = 'true'
  style.textContent = katexShadowCss
  root.prepend(style)
}

function upsertKatexShadowContent(root: ShadowRoot, html: string) {
  let content = root.querySelector<HTMLElement>('[data-katex-shadow-content="true"]')
  if (!content) {
    content = document.createElement('span')
    content.dataset.katexShadowContent = 'true'
    root.appendChild(content)
  }
  content.innerHTML = html
}

// KaTeX HTML is visually high fidelity but expands into hundreds of spans. Keep
// those spans out of the page-level DOM so WebKit global UI invalidations do not
// repeatedly match styles through every formula descendant.
function ShadowKatex({ html, display }: { html: string; display: boolean }) {
  const hostRef = useRef<HTMLSpanElement>(null)
  const renderedHtmlRef = useRef<string | null>(null)

  useLayoutEffect(() => {
    const host = hostRef.current
    if (!host) return
    const root = host.shadowRoot ?? host.attachShadow({ mode: 'open' })
    installKatexShadowStyles(root)
    if (renderedHtmlRef.current !== html) {
      upsertKatexShadowContent(root, html)
      renderedHtmlRef.current = html
    }
  }, [html])

  const cls = display ? 'katex-lazy katex-lazy--display' : 'katex-lazy'
  return <span ref={hostRef} className={cls} data-katex-shadow-host="true" />
}

// 按 (tex, display) 缓存 KaTeX 渲染结果：流式时每帧重渲会对每个公式重复调用，
// 同一公式只算一次。简单上限防无界增长(超了清空)。
const texCache = new Map<string, string>()
function renderTex(tex: string, display: boolean): string {
  const key = (display ? 'd:' : 'i:') + tex
  const cached = texCache.get(key)
  if (cached != null) return cached
  let out = ''
  try {
    const rendered = katex.renderToString(tex, { displayMode: display, throwOnError: false, output: 'html' })
    out = rendered.includes('katex-error') ? '' : rendered
  } catch {
    out = ''
  }
  if (texCache.size > 500) texCache.clear()
  texCache.set(key, out)
  return out
}

function LazyMath({ tex, display }: { tex: string; display: boolean }) {
  // 即时渲染（不再用 IntersectionObserver 延迟到滚动进视口才渲染）。KaTeX 子树进入
  // Shadow DOM，避免已完成公式让 WebKit 后续全局 UI 交互反复扫大段普通 DOM。
  const html = useMemo(() => renderTex(tex, display), [tex, display])
  if (html) {
    return <ShadowKatex html={html} display={display} />
  }
  const cls = display ? 'katex-lazy katex-lazy--display' : 'katex-lazy'
  return <span className={`${cls} katex-lazy--pending`}>{tex}</span>
}

// 模块级稳定组件：remark-math 产出的 <kvmath> → <LazyMath>。无闭包依赖，必须放模块级——
// 若写成 components useMemo 里的内联函数，每次重建 components（artifacts/citations 变化、或流式每帧）
// 都是新函数类型，ReactMarkdown 会把 LazyMath 整个卸载重挂（公式 remount 闪烁）。
function KvMath({ node }: { node?: { properties?: { tex?: string; display?: string } } }) {
  const props = node?.properties ?? {}
  return <LazyMath tex={String(props.tex ?? '')} display={props.display === 'true'} />
}

// remark-math 产出的 math/inlineMath 节点 → 自定义 <kvmath> 元素(携带 tex + display)，
// 由下方 components 的 kvmath 映射到 <LazyMath>。替代 rehype-katex 的即时渲染。
const remarkRehypeOptions = {
  handlers: {
    math: (_state: unknown, node: { value?: string }) => ({
      type: 'element',
      tagName: 'kvmath',
      properties: { display: 'true', tex: node.value ?? '' },
      children: [],
    }),
    inlineMath: (_state: unknown, node: { value?: string }) => ({
      type: 'element',
      tagName: 'kvmath',
      properties: { display: 'false', tex: node.value ?? '' },
      children: [],
    }),
  },
}

function ChatMarkdownComponent({
  content,
  artifacts = [],
  onImageClick,
  variant = 'default',
  citations,
}: ChatMarkdownProps) {
  const normalized = useMemo(() => normalizeMarkdownForRender(content), [content])
  const remarkPlugins = useMemo<PluggableList>(() => {
    const plugins: PluggableList = [remarkGfm, remarkMath]
    if (citations && citations.size > 0) {
      plugins.push(remarkCitations(new Set(citations.keys())))
    }
    return plugins
  }, [citations])
  const components = useMemo<Components>(() => {
    const artifactLookup = buildArtifactLookup(artifacts)
    return {
      ...markdownComponents,
      kvmath: KvMath,
      a: ({ href, children }) => {
        const url = typeof href === 'string' ? href : ''
        const cite = /^#kb-cite-(\d{1,3})$/.exec(url)
        if (cite) {
          const n = Number(cite[1])
          return <CitationChip n={n} hit={citations?.get(n)} />
        }
        return <LinkAnchor href={url}>{children}</LinkAnchor>
      },
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
  }, [artifacts, onImageClick, citations])

  return (
    <div className={variant === 'reasoning' ? reasoningProseClass : proseClass}>
      <MarkdownErrorBoundary fallbackText={content}>
        <ReactMarkdown
          remarkPlugins={remarkPlugins}
          remarkRehypeOptions={remarkRehypeOptions as never}
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
