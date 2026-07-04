import { useCallback, useEffect, useMemo, useState } from 'react'
import { RefreshCw, Trash2, Download, Copy, Check, ChevronRight, Search, Terminal, FileJson } from 'lucide-react'
import { api, type RequestDebugRecord } from '../api/tauri'
import { SettingRow, SettingsGroup, Toggle } from './components'

type RequestDebugPanelProps = {
  lang: string
  /** 當前開關值（來自 chatTools.requestDebugEnabled）。 */
  enabled: boolean
  /** 切換開關：更新 chatTools.requestDebugEnabled（使用者按儲存後生效）。 */
  onToggleEnabled: (enabled: boolean) => void
}

function formatTime(seconds?: number | null, lang = 'zh') {
  if (!seconds) return '--'
  return new Date(seconds * 1000).toLocaleTimeString(lang.startsWith('zh') ? lang : 'en-US', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
}

function formatDuration(ms?: number | null) {
  const n = Number(ms ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '--'
  if (n >= 1000) return `${(n / 1000).toFixed(1)}s`
  return `${Math.round(n)}ms`
}

/** 相鄰記錄的時間間隔（秒）轉成人類可讀的 gap 文案，供列表分隔用。 */
function formatGap(seconds: number, lang: string): string {
  if (seconds >= 3600) {
    const h = Math.floor(seconds / 3600)
    return lang.startsWith('zh') ? `${h} 小時間隔` : `${h}h gap`
  }
  if (seconds >= 60) {
    const m = Math.floor(seconds / 60)
    return lang.startsWith('zh') ? `${m} 分鐘間隔` : `${m} min gap`
  }
  return lang.startsWith('zh') ? `${Math.round(seconds)} 秒間隔` : `${Math.round(seconds)}s gap`
}

function totalTokens(record: RequestDebugRecord) {
  const usage = record.response.usage
  if (!usage) return null
  return (
    usage.totalTokens ??
    ((usage.inputTokens ?? 0) + (usage.outputTokens ?? 0) || null)
  )
}

/** 從完整 URL 抽出 `METHOD /path`，解析失敗時退回原串。 */
function endpointLabel(url: string): string {
  try {
    const u = new URL(url)
    return `POST ${u.pathname}`
  } catch {
    // 相對/異常 URL：盡力擷取 path 段
    const idx = url.indexOf('/', url.indexOf('://') + 3)
    return `POST ${idx >= 0 ? url.slice(idx) : url}`
  }
}

/** 列表項左側豎條顏色：按來源分類著色（error 恆為紅），呼應 claude-tap 的任務色條。 */
function sourceAccent(source: string): string {
  const s = source.toLowerCase()
  if (s.startsWith('chat')) {
    if (s.includes('title')) return 'border-l-teal-400'
    if (s.includes('compression')) return 'border-l-fuchsia-400'
    if (s.includes('vision') || s.includes('image')) return 'border-l-pink-400'
    return 'border-l-sky-500'
  }
  if (s.includes('translator')) return 'border-l-amber-500'
  if (s.includes('screenshot')) return 'border-l-violet-500'
  if (s.includes('lens')) return 'border-l-cyan-500'
  return 'border-l-neutral-300 dark:border-l-neutral-700'
}

/** 模型徽標配色：按模型家族關鍵字著色。 */
function modelBadgeClass(model: string): string {
  const l = model.toLowerCase()
  if (l.includes('claude') || l.includes('opus') || l.includes('sonnet') || l.includes('haiku'))
    return 'bg-violet-500/10 text-violet-600 dark:text-violet-300'
  if (l.includes('gpt') || /\bo[13]\b/.test(l)) return 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-300'
  if (l.includes('gemini')) return 'bg-sky-500/10 text-sky-600 dark:text-sky-300'
  if (l.includes('grok')) return 'bg-orange-500/10 text-orange-600 dark:text-orange-300'
  return 'bg-neutral-500/10 text-neutral-500 dark:text-neutral-400'
}

/** 來源類別徽標：短標籤 + 配色（與左側豎條同色系），未知來源退回原串。 */
const SOURCE_BADGE: Record<string, { zh: string; en: string; cls: string }> = {
  chat: { zh: 'Chat', en: 'Chat', cls: 'bg-sky-500/10 text-sky-600 dark:text-sky-300' },
  chat_title_summary: { zh: '標題', en: 'Title', cls: 'bg-teal-500/10 text-teal-600 dark:text-teal-300' },
  chat_compression: { zh: '壓縮', en: 'Compress', cls: 'bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300' },
  chat_aux_vision: { zh: '視覺', en: 'Vision', cls: 'bg-pink-500/10 text-pink-600 dark:text-pink-300' },
  chat_image_generation: { zh: '繪圖', en: 'Image', cls: 'bg-rose-500/10 text-rose-600 dark:text-rose-300' },
  translator: { zh: '翻譯', en: 'Translate', cls: 'bg-amber-500/10 text-amber-600 dark:text-amber-300' },
  screenshot_translation: { zh: '截圖', en: 'Shot', cls: 'bg-violet-500/10 text-violet-600 dark:text-violet-300' },
  lens: { zh: 'Lens', en: 'Lens', cls: 'bg-cyan-500/10 text-cyan-600 dark:text-cyan-300' },
}

function sourceBadge(source: string, lang: string): { label: string; cls: string } {
  const m = SOURCE_BADGE[source]
  if (m) return { label: lang.startsWith('zh') ? m.zh : m.en, cls: m.cls }
  return { label: source, cls: 'bg-neutral-500/10 text-neutral-500 dark:text-neutral-400' }
}



function prettyJson(value: unknown) {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

// ── YAML 序列化（Trace 檢視的 YAML 格式，對齊 claude-tap toTraceYaml）──────
function isScalar(v: unknown): boolean {
  return v === null || v === undefined || typeof v !== 'object'
}

function yamlKey(key: string): string {
  return /^[A-Za-z_][A-Za-z0-9_-]*$/.test(key) ? key : JSON.stringify(key)
}

function yamlScalar(value: unknown, indent: number): string {
  if (value === null || value === undefined) return 'null'
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  const str = String(value)
  if (str === '') return '""'
  if (str.includes('\n')) {
    const pad = ' '.repeat(indent + 2)
    return `|\n${str.split('\n').map((line) => pad + line).join('\n')}`
  }
  if (/^[A-Za-z0-9_./:@+-]+$/.test(str) && !/^(true|false|null|yes|no|on|off)$/i.test(str)) return str
  return JSON.stringify(str)
}

function toYaml(value: unknown, indent = 0): string {
  const pad = ' '.repeat(indent)
  if (isScalar(value)) return pad + yamlScalar(value, indent)
  if (Array.isArray(value)) {
    if (!value.length) return pad + '[]'
    return value
      .map((item) => (isScalar(item) ? `${pad}- ${yamlScalar(item, indent)}` : `${pad}-\n${toYaml(item, indent + 2)}`))
      .join('\n')
  }
  const obj = value as Record<string, unknown>
  const keys = Object.keys(obj).filter((k) => obj[k] !== undefined)
  if (!keys.length) return pad + '{}'
  return keys
    .map((key) =>
      isScalar(obj[key])
        ? `${pad}${yamlKey(key)}: ${yamlScalar(obj[key], indent)}`
        : `${pad}${yamlKey(key)}:\n${toYaml(obj[key], indent + 2)}`,
    )
    .join('\n')
}

/** 按當前格式（json/yaml）把負載轉成文字。 */
function formatPayload(value: unknown, mode: 'json' | 'yaml'): string {
  return mode === 'yaml' ? toYaml(value) : prettyJson(value)
}

function isObj(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v)
}

/** content 可能是 string 或 OpenAI/Anthropic 的 block 陣列，盡力拼成純文字。 */
function contentToText(content: unknown): string {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    return content
      .map((block) => {
        if (typeof block === 'string') return block
        if (isObj(block)) return (block.text as string) ?? (block.content as string) ?? ''
        return ''
      })
      .filter(Boolean)
      .join('\n')
  }
  if (isObj(content) && typeof content.text === 'string') return content.text
  return ''
}

/**
 * 從請求 body 裡拆出系統提示詞 / 訊息陣列，相容三種 wire 格式：
 * - OpenAI chat：messages[]（system 混在其中）
 * - Anthropic：system（string 或 block[]）+ messages[]
 * - Responses：instructions + input[]
 */
function splitRequestBody(body: unknown): { system: string | null; messages: unknown[] | null } {
  if (!isObj(body)) return { system: null, messages: null }
  let system: string | null = null
  let messages: unknown[] | null = null

  if (typeof body.system === 'string') system = body.system
  else if (Array.isArray(body.system)) system = body.system.map(contentToText).join('\n')

  if (Array.isArray(body.messages)) {
    messages = body.messages
    if (system == null) {
      const sys = body.messages.filter((m) => isObj(m) && m.role === 'system')
      if (sys.length) system = sys.map((m) => contentToText((m as Record<string, unknown>).content)).join('\n')
    }
  } else if (Array.isArray(body.input)) {
    messages = body.input
  }

  if (system == null && typeof body.instructions === 'string') system = body.instructions
  return { system, messages }
}

/** 工具定義顯示名：相容 Anthropic(name) / OpenAI(function.name) / 退回 id/type。 */
function toolDisplayName(td: unknown): string {
  if (!isObj(td)) return ''
  const fn = isObj(td.function) ? td.function : null
  for (const v of [td.name, fn?.name, td.id, td.type]) {
    if (typeof v === 'string' && v) return v
  }
  return ''
}

/** 工具描述：Anthropic(description) / OpenAI(function.description)。 */
function toolDescription(td: unknown): string {
  if (!isObj(td)) return ''
  const fn = isObj(td.function) ? td.function : null
  const desc = td.description ?? fn?.description
  return typeof desc === 'string' ? desc : ''
}

/** 工具入參 schema：Anthropic(input_schema) / OpenAI(parameters or function.parameters)。 */
function toolSchema(td: unknown): { properties?: Record<string, unknown>; required?: string[] } {
  if (!isObj(td)) return {}
  const fn = isObj(td.function) ? td.function : null
  const schema = td.input_schema ?? td.parameters ?? fn?.parameters
  return isObj(schema) ? (schema as { properties?: Record<string, unknown>; required?: string[] }) : {}
}

/** 從請求 body 裡取工具陣列（body.tools），非陣列時返回空。 */
function getRequestTools(body: unknown): unknown[] {
  if (!isObj(body)) return []
  return Array.isArray(body.tools) ? body.tools : []
}

/** 拼 curl 命令（headers 已脫敏，body 為記錄時的原樣）。 */
function buildCurl(request: RequestDebugRecord['request']): string {
  const lines = [`curl -X POST '${request.url}'`]
  for (const [key, value] of Object.entries(request.headers)) {
    lines.push(`  -H '${key}: ${value}'`)
  }
  const bodyStr = typeof request.body === 'string' ? request.body : prettyJson(request.body)
  lines.push(`  -d '${bodyStr.replace(/'/g, "'\\''")}'`)
  return lines.join(' \\\n')
}

function CopyButton({ text, lang, label }: { text: string; lang: string; label?: string }) {
  const [copied, setCopied] = useState(false)
  const copy = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation()
      try {
        await navigator.clipboard.writeText(text)
        setCopied(true)
        setTimeout(() => setCopied(false), 1400)
      } catch {
        /* clipboard unavailable — ignore */
      }
    },
    [text],
  )
  return (
    <button type="button" className="kv-btn sm" onClick={copy} data-tauri-drag-region="false">
      {copied ? <Check size={11} /> : <Copy size={11} />}
      {copied ? (lang.startsWith('zh') ? '已複製' : 'Copied') : label ?? (lang.startsWith('zh') ? '複製' : 'Copy')}
    </button>
  )
}

/** 可摺疊分割槽：標題 + 可選計數徽標 + 複製按鈕 + 展開內容。 */
function Section({
  title,
  badge,
  copyText,
  defaultOpen = false,
  lang,
  children,
}: {
  title: string
  badge?: string
  copyText?: string
  defaultOpen?: boolean
  lang: string
  children: React.ReactNode
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
      <div className="flex items-center justify-between gap-2 px-3 py-2">
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          data-tauri-drag-region="false"
        >
          <ChevronRight
            size={13}
            className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-90' : ''}`}
            strokeWidth={2.25}
          />
          <span className="truncate text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">{title}</span>
          {badge && <span className="kv-chip shrink-0">{badge}</span>}
        </button>
        {copyText != null && <CopyButton text={copyText} lang={lang} />}
      </div>
      {open && <div className="border-t border-neutral-200 dark:border-neutral-800">{children}</div>}
    </div>
  )
}

/** 單個工具：青色 mono 名 + 灰色短描述，展開後顯示完整描述 + 引數列表。 */
function ToolBlock({ td, lang }: { td: unknown; lang: string }) {
  const [open, setOpen] = useState(false)
  const name = toolDisplayName(td) || 'unknown'
  const desc = toolDescription(td)
  const shortDesc = desc.split('\n')[0].slice(0, 140)
  const schema = toolSchema(td)
  const props = isObj(schema.properties) ? schema.properties : {}
  const required = new Set(Array.isArray(schema.required) ? schema.required : [])
  const paramKeys = Object.keys(props)
  return (
    <div className="border-b border-neutral-100 last:border-b-0 dark:border-neutral-900">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-neutral-50 dark:hover:bg-neutral-900/40"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        data-tauri-drag-region="false"
      >
        <ChevronRight
          size={11}
          className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-90' : ''}`}
          strokeWidth={2.25}
        />
        <span className="shrink-0 font-mono text-[12px] font-semibold text-cyan-600 dark:text-cyan-400">{name}</span>
        <span className="min-w-0 flex-1 truncate text-[12px] text-neutral-400">{shortDesc}</span>
      </button>
      {open && (
        <div className="px-3 pb-3 pl-8">
          {desc && (
            <div className="mb-2 whitespace-pre-wrap break-words text-[11px] leading-relaxed text-neutral-600 dark:text-neutral-300">
              {desc}
            </div>
          )}
          {paramKeys.length > 0 && (
            <>
              <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
                {lang.startsWith('zh') ? '引數' : 'Parameters'}
              </div>
              <div className="flex flex-col gap-1.5">
                {paramKeys.map((key) => {
                  const p = isObj(props[key]) ? (props[key] as Record<string, unknown>) : {}
                  const type = (p.type as string) || (Array.isArray(p.enum) ? 'enum' : '')
                  const pdesc = typeof p.description === 'string' ? p.description : ''
                  return (
                    <div key={key}>
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="font-mono text-[12px] font-semibold text-sky-600 dark:text-sky-400">{key}</span>
                        {type && (
                          <span className="rounded bg-neutral-100 px-1 py-0.5 font-mono text-[10px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                            {type}
                          </span>
                        )}
                        {required.has(key) && (
                          <span className="rounded bg-amber-500/10 px-1 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                            {lang.startsWith('zh') ? '必填' : 'required'}
                          </span>
                        )}
                      </div>
                      {pdesc && (
                        <div className="mt-0.5 text-[11px] leading-snug text-neutral-500 dark:text-neutral-400">{pdesc}</div>
                      )}
                    </div>
                  )
                })}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}

function JsonBody({ value }: { value: unknown }) {
  const text = typeof value === 'string' ? value : prettyJson(value)
  return (
    <pre className="max-h-[360px] overflow-auto px-3 py-2 text-[11px] leading-relaxed text-neutral-700 dark:text-neutral-300 whitespace-pre-wrap break-all font-mono">
      {text}
    </pre>
  )
}

// ── 訊息渲染（對齊 claude-tap 的角色卡片 + block 檢視）──────────────────

type MsgBlock = { type?: string; [k: string]: unknown }

/** OpenAI tool_call.arguments 可能是 JSON 字串，盡力 parse。 */
function parseToolArgs(args: unknown): unknown {
  if (args == null || args === '') return {}
  if (typeof args !== 'string') return args
  try {
    return JSON.parse(args)
  } catch {
    return args
  }
}

/** 把 content（string / block[] / object）歸一化成 block 陣列。 */
function toDisplayBlocks(content: unknown): MsgBlock[] {
  if (typeof content === 'string') return content.trim() ? [{ type: 'text', text: content }] : []
  if (content == null) return []
  if (!Array.isArray(content)) return [{ type: 'raw', value: content }]
  return content.map((b) => {
    if (typeof b === 'string') return { type: 'text', text: b }
    if (!isObj(b)) return { type: 'raw', value: b }
    return b as MsgBlock
  })
}

/**
 * 歸一化一條訊息為 { role, blocks }，相容：
 * - OpenAI：role:'tool' → tool_result；assistant.tool_calls[] → tool_use；string content → text
 * - Anthropic：content 已是 block 陣列，直接用
 */
function normalizeMessage(msg: unknown): { role: string; blocks: MsgBlock[] } {
  if (!isObj(msg)) return { role: 'unknown', blocks: [] }
  const role = (msg.role as string) || 'unknown'
  if (role === 'tool') {
    return {
      role,
      blocks: [{ type: 'tool_result', tool_use_id: (msg.tool_call_id as string) || '', content: msg.content ?? '' }],
    }
  }
  const raw: unknown[] = []
  if (Array.isArray(msg.content)) raw.push(...msg.content)
  else if (typeof msg.content === 'string') {
    if (msg.content.trim()) raw.push({ type: 'text', text: msg.content })
  } else if (msg.content != null) {
    raw.push(msg.content)
  }
  if (Array.isArray(msg.tool_calls)) {
    for (const call of msg.tool_calls) {
      const c = isObj(call) ? call : {}
      const fn = isObj(c.function) ? c.function : {}
      raw.push({
        type: 'tool_use',
        id: (c.id as string) || '',
        name: (fn.name as string) || (c.name as string) || 'tool_use',
        input: parseToolArgs(fn.arguments),
      })
    }
  }
  return { role, blocks: toDisplayBlocks(raw) }
}

const MSG_ROLE_STYLE: Record<string, { badge: string; card: string }> = {
  user: {
    badge: 'bg-sky-500 text-white',
    card: 'border-sky-200 bg-sky-50/60 dark:border-sky-900/40 dark:bg-sky-950/20',
  },
  assistant: {
    badge: 'bg-emerald-500 text-white',
    card: 'border-emerald-200 bg-emerald-50/50 dark:border-emerald-900/40 dark:bg-emerald-950/20',
  },
  tool: {
    badge: 'bg-violet-500 text-white',
    card: 'border-violet-200 bg-violet-50/40 dark:border-violet-900/40 dark:bg-violet-950/20',
  },
  system: {
    badge: 'bg-neutral-500 text-white',
    card: 'border-neutral-200 bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900/40',
  },
}

function InlinePre({ text }: { text: string }) {
  return (
    <pre className="mt-1 max-h-[280px] overflow-auto rounded-md border border-neutral-200 bg-white/70 px-3 py-2 text-[11px] leading-relaxed whitespace-pre-wrap break-all font-mono text-neutral-700 dark:border-neutral-800 dark:bg-neutral-950/40 dark:text-neutral-300">
      {text}
    </pre>
  )
}

function MessageBlock({ block }: { block: MsgBlock }) {
  const type = block.type
  if (type === 'text' || type === 'input_text' || type === 'output_text') {
    const txt = String(block.text ?? '')
    if (!txt.trim()) return null
    return <div className="whitespace-pre-wrap break-words text-[12px] leading-relaxed text-neutral-700 dark:text-neutral-200">{txt}</div>
  }
  if (type === 'thinking') {
    const txt = String(block.thinking ?? '')
    if (!txt.trim()) return null
    return (
      <div>
        <span className="text-[10px] font-semibold uppercase tracking-wide text-amber-600 dark:text-amber-400">thinking</span>
        <InlinePre text={txt} />
      </div>
    )
  }
  if (type === 'tool_use') {
    const name = String(block.name ?? 'tool_use')
    const id = String(block.id ?? '')
    return (
      <div>
        <span className="font-mono text-[11px] font-semibold text-cyan-600 dark:text-cyan-400">
          {id ? `${name} (${id})` : name}
        </span>
        <InlinePre text={prettyJson(block.input)} />
      </div>
    )
  }
  if (type === 'tool_result') {
    const id = String(block.tool_use_id ?? '')
    const rc = block.content
    let text: string
    if (typeof rc === 'string') text = rc
    else if (Array.isArray(rc)) {
      text = rc
        .map((c) => (isObj(c) && typeof c.text === 'string' ? c.text : prettyJson(c)))
        .join('\n')
    } else text = prettyJson(rc)
    return (
      <div>
        <span className="font-mono text-[11px] font-semibold text-cyan-600 dark:text-cyan-400">
          {id ? `result (${id})` : 'result'}
        </span>
        <InlinePre text={text} />
      </div>
    )
  }
  // raw / 未知 block
  return <InlinePre text={prettyJson('value' in block ? block.value : block)} />
}

function MessageCard({ msg }: { msg: unknown }) {
  const { role, blocks } = useMemo(() => normalizeMessage(msg), [msg])
  const rendered = blocks.map((b, i) => <MessageBlock key={i} block={b} />).filter(Boolean)
  if (rendered.length === 0) return null
  const style = MSG_ROLE_STYLE[role] ?? MSG_ROLE_STYLE.system
  return (
    <div className={`rounded-lg border p-2.5 ${style.card}`}>
      <span className={`inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide ${style.badge}`}>
        {role}
      </span>
      <div className="mt-2 flex flex-col gap-2">{rendered}</div>
    </div>
  )
}

function MessagesView({ messages }: { messages: unknown[] }) {
  return (
    <div className="flex max-h-[520px] flex-col gap-2 overflow-auto px-3 py-2">
      {messages.map((m, i) => (
        <MessageCard key={i} msg={m} />
      ))}
    </div>
  )
}

const USAGE_ITEMS: Array<{
  key: keyof NonNullable<NonNullable<RequestDebugRecord['response']['usage']>>
  zh: string
  en: string
  dot: string
}> = [
  { key: 'inputTokens', zh: '輸入', en: 'Input', dot: 'bg-sky-500' },
  { key: 'outputTokens', zh: '輸出', en: 'Output', dot: 'bg-emerald-500' },
  { key: 'cachedInputTokens', zh: '快取讀取', en: 'Cache read', dot: 'bg-amber-500' },
  { key: 'cacheCreationInputTokens', zh: '快取建立', en: 'Cache write', dot: 'bg-violet-500' },
  { key: 'reasoningTokens', zh: '推理', en: 'Reasoning', dot: 'bg-rose-500' },
]

function UsageBar({ record, lang }: { record: RequestDebugRecord; lang: string }) {
  const usage = record.response.usage
  if (!usage) return null
  const items = USAGE_ITEMS.filter((item) => usage[item.key] != null)
  if (items.length === 0) return null
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px]">
      {items.map((item) => (
        <span key={item.key} className="inline-flex items-center gap-1.5">
          <span className={`size-1.5 rounded-full ${item.dot}`} />
          <span className="text-neutral-500 dark:text-neutral-400">{lang.startsWith('zh') ? item.zh : item.en}</span>
          <span className="tabular-nums font-medium text-neutral-700 dark:text-neutral-200">
            {(usage[item.key] as number).toLocaleString()}
          </span>
        </span>
      ))}
    </div>
  )
}

export function RequestDebugPanel({ lang, enabled, onToggleEnabled }: RequestDebugPanelProps) {
  const zh = lang.startsWith('zh')
  const [records, setRecords] = useState<RequestDebugRecord[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [query, setQuery] = useState('')

  const refresh = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      const next = await api.getRequestDebugRecords()
      setRecords(next)
      setSelectedId((prev) => {
        if (prev && next.some((r) => r.id === prev)) return prev
        return next[0]?.id ?? null
      })
    } catch (err) {
      setError(zh ? `載入失敗：${err}` : `Load failed: ${err}`)
    } finally {
      setLoading(false)
    }
  }, [zh])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const clearAll = useCallback(async () => {
    try {
      await api.clearRequestDebugRecords()
      setRecords([])
      setSelectedId(null)
    } catch (err) {
      setError(zh ? `清空失敗：${err}` : `Clear failed: ${err}`)
    }
  }, [zh])

  const exportJson = useCallback(() => {
    const blob = new Blob([prettyJson(records)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    const stamp = new Date().toISOString().replace(/[:.]/g, '-')
    anchor.href = url
    anchor.download = `kivio-request-debug-${stamp}.json`
    document.body.appendChild(anchor)
    anchor.click()
    anchor.remove()
    URL.revokeObjectURL(url)
  }, [records])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return records
    return records.filter((r) =>
      [r.operation, r.model, r.providerName, r.providerId, r.request.url, r.source]
        .filter(Boolean)
        .some((field) => String(field).toLowerCase().includes(q)),
    )
  }, [records, query])

  const selected = useMemo(
    () => records.find((r) => r.id === selectedId) ?? null,
    [records, selectedId],
  )

  return (
    <div className="flex flex-col gap-4">
      <SettingsGroup title={zh ? '請求除錯' : 'Request debug'}>
        <SettingRow
          label={zh ? '記錄 provider 請求' : 'Capture provider requests'}
          description={
            zh
              ? '開啟後每次 provider 呼叫（chat/子agent + 翻譯/截圖/Lens）的請求與回應被記入記憶體（脫敏 key，最多 50 條，不落盤）。改動後請儲存設定生效。'
              : 'When on, every provider call (chat/sub-agent + translate/screenshot/Lens) is captured in memory (keys masked, up to 50, never written to disk). Save settings to apply.'
          }
          stack
        >
          <Toggle checked={enabled} onChange={onToggleEnabled} />
        </SettingRow>
      </SettingsGroup>

      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className="kv-btn sm" onClick={() => void refresh()} data-tauri-drag-region="false">
          <RefreshCw size={11} className={loading ? 'animate-spin' : ''} />
          {zh ? '重新整理' : 'Refresh'}
        </button>
        <button
          type="button"
          className="kv-btn sm"
          onClick={exportJson}
          disabled={records.length === 0}
          data-tauri-drag-region="false"
        >
          <Download size={11} />
          {zh ? '匯出 JSON' : 'Export JSON'}
        </button>
        <button
          type="button"
          className="kv-btn sm"
          onClick={() => void clearAll()}
          disabled={records.length === 0}
          data-tauri-drag-region="false"
        >
          <Trash2 size={11} />
          {zh ? '清空' : 'Clear'}
        </button>
        <span className="ml-auto text-[11px] text-neutral-500 dark:text-neutral-400">
          {zh ? `${records.length} 條記錄` : `${records.length} records`}
        </span>
      </div>

      {error && <div className="text-[11px] text-red-500">{error}</div>}

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-[minmax(240px,340px)_1fr]">
        {/* 左列：搜尋 + 請求列表 */}
        <div className="flex max-h-[560px] flex-col overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
          <div className="flex items-center gap-1.5 border-b border-neutral-200 px-2.5 py-1.5 dark:border-neutral-800">
            <Search size={12} className="shrink-0 text-neutral-400" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={zh ? '搜尋 operation / 模型 / URL…' : 'Search operation / model / URL…'}
              className="w-full bg-transparent text-[12px] text-neutral-700 outline-none placeholder:text-neutral-400 dark:text-neutral-200"
              data-tauri-drag-region="false"
              spellCheck={false}
            />
          </div>
          <div className="flex-1 overflow-auto">
            {filtered.length === 0 ? (
              <div className="px-3 py-6 text-center text-[11px] text-neutral-400">
                {records.length === 0
                  ? enabled
                    ? zh
                      ? '暫無記錄。觸發一次對話或翻譯後重新整理。'
                      : 'No records yet. Trigger a chat or translation, then refresh.'
                    : zh
                      ? '未開啟記錄。開啟上方開關並儲存。'
                      : 'Capture is off. Turn on the toggle above and save.'
                  : zh
                    ? '沒有匹配的記錄。'
                    : 'No matching records.'}
              </div>
            ) : (
              filtered.map((record, i) => {
                const tokens = totalTokens(record)
                const active = record.id === selectedId
                const ok = record.status === 'success'
                // gap 分隔：與列表中更新的上一條（i-1，時間更晚）比較
                const newer = filtered[i - 1]
                const gapSeconds = newer ? (newer.createdAt ?? 0) - (record.createdAt ?? 0) : 0
                const showGap = gapSeconds >= 120
                return (
                  <div key={record.id}>
                    {showGap && (
                      <div className="flex items-center gap-2 px-3 py-1">
                        <span className="h-px flex-1 bg-neutral-200 dark:bg-neutral-800" />
                        <span className="text-[10px] text-neutral-400">{formatGap(gapSeconds, lang)}</span>
                        <span className="h-px flex-1 bg-neutral-200 dark:bg-neutral-800" />
                      </div>
                    )}
                    <button
                      type="button"
                      onClick={() => setSelectedId(record.id)}
                      data-tauri-drag-region="false"
                      className={`flex w-full flex-col gap-1 border-b border-l-[3px] border-neutral-100 px-3 py-2.5 text-left last:border-b-0 dark:border-neutral-900 ${
                        ok ? sourceAccent(record.source) : 'border-l-red-500'
                      } ${
                        active
                          ? 'bg-sky-50 dark:bg-sky-950/30'
                          : 'hover:bg-neutral-50 dark:hover:bg-neutral-900/40'
                      }`}
                    >
                      <div className="flex items-center gap-1.5">
                        <span
                          className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold ${sourceBadge(record.source, lang).cls}`}
                        >
                          {sourceBadge(record.source, lang).label}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-neutral-800 dark:text-neutral-100">
                          {record.operation}
                        </span>
                        {!ok && (
                          <span
                            className="size-2 shrink-0 rounded-full bg-red-500 ring-2 ring-red-500/20"
                            title={`status: ${record.status}`}
                          />
                        )}
                        <span
                          className={`ml-auto max-w-[45%] shrink-0 truncate rounded px-1.5 py-0.5 text-[10px] font-medium ${modelBadgeClass(record.model)}`}
                          title={record.model}
                        >
                          {record.model}
                        </span>
                      </div>
                      <div className="flex items-center gap-2.5 font-mono text-[11px] tabular-nums">
                        <span className="font-medium text-emerald-600 dark:text-emerald-400">
                          {(tokens ?? 0).toLocaleString()} tok
                        </span>
                        <span className="font-medium text-amber-600 dark:text-amber-500">
                          {formatDuration(record.durationMs)}
                        </span>
                        <span className="ml-auto text-neutral-400">{formatTime(record.createdAt, lang)}</span>
                      </div>
                      <div className="truncate font-mono text-[10px] text-neutral-400" title={record.request.url}>
                        {endpointLabel(record.request.url)}
                      </div>
                    </button>
                  </div>
                )
              })
            )}
          </div>
        </div>

        {/* 右列：詳情 */}
        <div className="flex flex-col gap-3">
          {selected ? (
            <DetailView selected={selected} lang={lang} />
          ) : (
            <div className="rounded-md border border-dashed border-neutral-200 px-3 py-10 text-center text-[11px] text-neutral-400 dark:border-neutral-800">
              {zh ? '選擇左側一條記錄檢視詳情。' : 'Select a record on the left to view details.'}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function DetailView({ selected, lang }: { selected: RequestDebugRecord; lang: string }) {
  const zh = lang.startsWith('zh')
  const [view, setView] = useState<'default' | 'trace'>('default')
  const { system, messages } = useMemo(() => splitRequestBody(selected.request.body), [selected])
  const tools = useMemo(() => getRequestTools(selected.request.body), [selected])
  const curl = useMemo(() => buildCurl(selected.request), [selected])
  const requestBodyText = useMemo(
    () =>
      typeof selected.request.body === 'string'
        ? selected.request.body
        : prettyJson(selected.request.body),
    [selected],
  )

  return (
    <>
      {/* 檢視切換：預設 / Trace */}
      <div className="kv-seg w-fit">
        <button
          type="button"
          className={view === 'default' ? 'active' : ''}
          onClick={() => setView('default')}
          data-tauri-drag-region="false"
        >
          {zh ? '預設' : 'Default'}
        </button>
        <button
          type="button"
          className={view === 'trace' ? 'active' : ''}
          onClick={() => setView('trace')}
          data-tauri-drag-region="false"
        >
          Trace
        </button>
      </div>

      {/* 元資訊 */}
      <div className="grid grid-cols-2 gap-x-4 gap-y-1 rounded-md border border-neutral-200 px-3 py-2 text-[11px] dark:border-neutral-800 sm:grid-cols-3">
        <Meta label={zh ? '供應商' : 'Provider'} value={`${selected.providerName} (${selected.providerId})`} />
        <Meta label={zh ? '模型' : 'Model'} value={selected.model} />
        <Meta label={zh ? '格式' : 'Format'} value={selected.apiFormat} />
        <Meta label={zh ? '來源' : 'Source'} value={selected.source} />
        <Meta label={zh ? '流式' : 'Stream'} value={selected.request.stream ? 'true' : 'false'} />
        <Meta label={zh ? '狀態碼' : 'Status'} value={String(selected.response.statusCode ?? '--')} />
        <Meta label={zh ? '耗時' : 'Duration'} value={formatDuration(selected.durationMs)} />
        {selected.response.finishReason && (
          <Meta label={zh ? '結束原因' : 'Finish'} value={selected.response.finishReason} />
        )}
        {selected.conversationId && <Meta label={zh ? '會話' : 'Conversation'} value={selected.conversationId} />}
      </div>

      {view === 'trace' ? (
        <TraceView selected={selected} system={system} messages={messages} tools={tools} lang={lang} />
      ) : (
        <>
          {/* endpoint + 操作按鈕 */}
          <div className="flex flex-wrap items-center gap-2">
            <span className="mr-auto truncate font-mono text-[11px] text-neutral-500 dark:text-neutral-400" title={selected.request.url}>
              {endpointLabel(selected.request.url)}
            </span>
            <CopyButton text={requestBodyText} lang={lang} label={zh ? '請求 JSON' : 'Request JSON'} />
            <CopyButtonIcon text={curl} lang={lang} icon={<Terminal size={11} />} label="cURL" />
            <CopyButtonIcon
              text={prettyJson(selected)}
              lang={lang}
              icon={<FileJson size={11} />}
              label={zh ? '整條' : 'Record'}
            />
          </div>

          {/* usage 明細 */}
          <UsageBar record={selected} lang={lang} />

          {/* 可摺疊分割槽：工具 → 系統提示詞 → 訊息 → 回應 → 請求 Body/Headers → 完整 JSON */}
          {tools.length > 0 && (
            <Section
              title={zh ? '工具' : 'Tools'}
              badge={zh ? `${tools.length} 個工具` : `${tools.length} tools`}
              copyText={prettyJson(tools)}
              lang={lang}
            >
              <div>
                {tools.map((td, i) => (
                  <ToolBlock key={`${toolDisplayName(td) || 'tool'}-${i}`} td={td} lang={lang} />
                ))}
              </div>
            </Section>
          )}
          {system && (
            <Section title={zh ? '系統提示詞' : 'System prompt'} copyText={system} lang={lang}>
              <pre className="max-h-[360px] overflow-auto px-3 py-2 text-[11px] leading-relaxed text-neutral-700 dark:text-neutral-300 whitespace-pre-wrap break-words font-mono">
                {system}
              </pre>
            </Section>
          )}
          {messages && (
            <Section
              title={zh ? '訊息' : 'Messages'}
              badge={zh ? `${messages.length} 條訊息` : `${messages.length} messages`}
              copyText={prettyJson(messages)}
              lang={lang}
            >
              <MessagesView messages={messages} />
            </Section>
          )}
          <Section title={zh ? '回應' : 'Response'} defaultOpen copyText={prettyJson(selected.response)} lang={lang}>
            <JsonBody value={selected.response} />
          </Section>
          <Section title={zh ? '請求 Body' : 'Request body'} copyText={requestBodyText} lang={lang}>
            <JsonBody value={selected.request.body} />
          </Section>
          <Section title={zh ? '請求 Headers（已脫敏）' : 'Request headers (masked)'} copyText={prettyJson(selected.request.headers)} lang={lang}>
            <JsonBody value={selected.request.headers} />
          </Section>
          <Section title={zh ? '完整 JSON' : 'Full JSON'} copyText={prettyJson(selected)} lang={lang}>
            <JsonBody value={selected} />
          </Section>
        </>
      )}
    </>
  )
}

/** Trace 檢視的單個資料塊：標題 + 徽標 + 複製 + 按當前格式渲染的負載。 */
function TraceBlock({
  title,
  badge,
  payload,
  mode,
  lang,
}: {
  title: string
  badge?: string
  payload: unknown
  mode: 'json' | 'yaml'
  lang: string
}) {
  const text = useMemo(() => formatPayload(payload, mode), [payload, mode])
  return (
    <div className="overflow-hidden rounded-md border border-neutral-200 dark:border-neutral-800">
      <div className="flex items-center justify-between gap-2 border-b border-neutral-200 px-3 py-2 dark:border-neutral-800">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="truncate text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">{title}</span>
          {badge && <span className="kv-chip shrink-0">{badge}</span>}
        </div>
        <CopyButton text={text} lang={lang} />
      </div>
      <pre className="max-h-[420px] overflow-auto px-3 py-2 text-[11px] leading-relaxed text-neutral-700 dark:text-neutral-300 whitespace-pre-wrap break-all font-mono">
        {text}
      </pre>
    </div>
  )
}

/**
 * Trace 檢視：把記錄按 輸入 / 輸出 / 後設資料 重新聚合成原始資料塊，
 * 支援 JSON / YAML 切換、整塊複製。SSE 逐幀未記錄（PRD 範圍外），故不含。
 */
function TraceView({
  selected,
  system,
  messages,
  tools,
  lang,
}: {
  selected: RequestDebugRecord
  system: string | null
  messages: unknown[] | null
  tools: unknown[]
  lang: string
}) {
  const zh = lang.startsWith('zh')
  const [mode, setMode] = useState<'json' | 'yaml'>('json')

  const inputPayload = useMemo(
    () => ({
      system: system ?? undefined,
      messages: messages ?? [],
      tools: tools ?? [],
    }),
    [system, messages, tools],
  )
  const outputPayload = useMemo(
    () => ({
      statusCode: selected.response.statusCode ?? null,
      finishReason: selected.response.finishReason ?? null,
      text: selected.response.text ?? null,
      reasoning: selected.response.reasoning ?? null,
      toolCalls: selected.response.toolCalls ?? null,
      usage: selected.response.usage ?? null,
      error: selected.response.error ?? null,
    }),
    [selected],
  )
  const metadata = useMemo(() => {
    let path = selected.request.url
    try {
      path = new URL(selected.request.url).pathname
    } catch {
      /* keep raw url */
    }
    return {
      id: selected.id,
      operation: selected.operation,
      source: selected.source,
      providerId: selected.providerId,
      providerName: selected.providerName,
      model: selected.model,
      apiFormat: selected.apiFormat,
      conversationId: selected.conversationId ?? null,
      messageId: selected.messageId ?? null,
      method: 'POST',
      path,
      url: selected.request.url,
      stream: selected.request.stream,
      status: selected.status,
      statusCode: selected.response.statusCode ?? null,
      durationMs: selected.durationMs,
      createdAt: selected.createdAt,
    }
  }, [selected])

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="kv-seg w-fit">
          <button
            type="button"
            className={mode === 'json' ? 'active' : ''}
            onClick={() => setMode('json')}
            data-tauri-drag-region="false"
          >
            JSON
          </button>
          <button
            type="button"
            className={mode === 'yaml' ? 'active' : ''}
            onClick={() => setMode('yaml')}
            data-tauri-drag-region="false"
          >
            YAML
          </button>
        </div>
        <CopyButtonIcon
          text={formatPayload({ input: inputPayload, output: outputPayload, metadata }, mode)}
          lang={lang}
          icon={<FileJson size={11} />}
          label={zh ? '整條' : 'All'}
        />
      </div>

      <UsageBar record={selected} lang={lang} />

      <TraceBlock
        title={zh ? '輸入' : 'Input'}
        badge={messages ? (zh ? `${messages.length} 條訊息` : `${messages.length} messages`) : undefined}
        payload={inputPayload}
        mode={mode}
        lang={lang}
      />
      <TraceBlock
        title={zh ? '輸出' : 'Output'}
        badge={String(selected.response.statusCode ?? selected.status)}
        payload={outputPayload}
        mode={mode}
        lang={lang}
      />
      <TraceBlock title={zh ? '後設資料' : 'Metadata'} payload={metadata} mode={mode} lang={lang} />
    </div>
  )
}

/** 帶自訂圖示的複製按鈕（cURL / 整條記錄）。 */
function CopyButtonIcon({
  text,
  lang,
  icon,
  label,
}: {
  text: string
  lang: string
  icon: React.ReactNode
  label: string
}) {
  const [copied, setCopied] = useState(false)
  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 1400)
    } catch {
      /* clipboard unavailable — ignore */
    }
  }, [text])
  return (
    <button type="button" className="kv-btn sm" onClick={copy} data-tauri-drag-region="false">
      {copied ? <Check size={11} /> : icon}
      {copied ? (lang.startsWith('zh') ? '已複製' : 'Copied') : label}
    </button>
  )
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-[10px] text-neutral-400">{label}</span>
      <span className="truncate text-neutral-700 dark:text-neutral-200" title={value}>
        {value}
      </span>
    </div>
  )
}
