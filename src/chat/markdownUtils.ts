/** Run `transform` only outside fenced/inline code so LaTeX/table fixes never touch code. */
function outsideCode(content: string, transform: (text: string) => string): string {
  // split keeps the code segments (odd indices) verbatim; even indices are prose.
  // The second alternative catches an **unterminated** fence: mid-stream a code block
  // has no closing ``` yet, and without this branch the table/LaTeX rewrites would
  // reach inside it and mangle the code the user is watching arrive.
  return content
    .split(/(```[\s\S]*?```|```[\s\S]*$|`[^`\n]*`)/g)
    .map((part, i) => (i % 2 === 0 ? transform(part) : part))
    .join('')
}

/** GFM 表格的分隔行（`| --- |`、`| :-: |`）。remark-gfm 见不到它就不认表格。 */
const TABLE_DELIMITER_ROW = /^\s*\|(\s*:?-+:?\s*\|)+\s*$/

/**
 * 模型常把整张 GFM 表挤在一行里（`| a | b | | c | d |`），而且经常漏掉分隔行。
 * 两种毛病 remark-gfm 都不认，表格退化成普通段落——段落又会把软换行折叠成空格，
 * 于是整张表糊成一长串竖线。这就是流式回答里最常见的「表格不渲染」。
 *
 * 做法：同一行内 `|` 紧跟 `|` 处断行；断出 ≥2 行且每行都以 `|` 开头，才认定它本来是
 * 一张被挤扁的表；首行后面若不是分隔行，按列数补一条。
 *
 * **断行只用 `[ \t]`，绝不用 `\s`**：`\s` 会跨换行匹配，把本来就正确的多行表格切出
 * 空行、拆成两个块（实测 rows 2→1）。这个坑踩过，别改回去。
 *
 * 只救「挤成一行」这一种。跨多行但缺分隔行的写法歧义太大——正常散文也可能连着几行以
 * `|` 开头——宁可不认，也不误伤。
 */
function fixSquashedTables(text: string): string {
  const out: string[] = []
  for (const line of text.split('\n')) {
    const rows = line.replace(/\|(?=[ \t]*\|)/g, '|\n').split('\n')
    const trimmed = rows.map((row) => row.trim())
    // 首行至少要有两根竖线（= 至少一个单元格）；其余行只要求以 `|` 开头，这样流式中
    // 尚未写完的末行（`| c`）不会让整张表忽然判定失败、闪一下消失。
    const squashed =
      trimmed.length >= 2 &&
      trimmed.every((row) => row.startsWith('|')) &&
      (trimmed[0].match(/\|/g)?.length ?? 0) >= 2
    if (!squashed) {
      out.push(line)
      continue
    }
    const columns = (trimmed[0].match(/\|/g)?.length ?? 0) - 1
    if (columns >= 1 && !TABLE_DELIMITER_ROW.test(trimmed[1])) {
      trimmed.splice(1, 0, `|${' --- |'.repeat(columns)}`)
    }
    out.push(...trimmed)
  }
  return out.join('\n')
}

/**
 * 送进 remark 之前的字符串级修补（对齐 Streamdown/remend 的思路：解析器不动，先把
 * 流式中不完整/不规范的写法补成合法 markdown）。
 *
 * - GFM 表格：挤成一行的拆回多行、缺分隔行的补上（见 `fixSquashedTables`）。
 * - LaTeX：`\[...\]` / `\(...\)` 转成 remark-math 认识的 `$$...$$` / `$...$`。
 *   非贪婪配对，未闭合的 `\[` 原样留着（流式中正确的行为）；行内公式要 trim，
 *   因为 remark-math 不接受 `$` 后面紧跟空白。
 */
export function normalizeMarkdownForRender(content: string): string {
  return outsideCode(content, (text) =>
    fixSquashedTables(
      text
        .replace(/\\\[([\s\S]*?)\\\]/g, (_m, body) => `\n$$\n${body.trim()}\n$$\n`)
        .replace(/\\\(([\s\S]*?)\\\)/g, (_m, body) => `$${body.trim()}$`),
    ),
  )
}

/**
 * Streamdown deliberately sanitizes file:/relative destinations before custom
 * link components receive them. Keep those two app-owned link kinds intact via
 * an internal URI scheme; ChatMarkdown decodes it immediately before handling
 * the click. Code spans/fences are excluded by the same outsideCode guard used
 * by the other Markdown repairs.
 */
export function preserveLocalMarkdownLinks(content: string): string {
  return outsideCode(content, (text) =>
    text.replace(/(!?\[[^\]]*\]\()([^\s)]+)(\))/g, (_match, prefix: string, destination: string, suffix: string) => {
      if (/^(?:https?:|mailto:|tel:|sms:|data:|blob:|tauri:|asset:|#)/i.test(destination)) {
        return `${prefix}${destination}${suffix}`
      }
      if (/^file:\/\//i.test(destination)) {
        return `${prefix}https://kivio.local/__kivio-file?target=${encodeURIComponent(destination)}${suffix}`
      }
      return `${prefix}https://kivio.local/__kivio-local?target=${encodeURIComponent(destination)}${suffix}`
    }),
  )
}

type MarkdownNode = {
  type?: string
  value?: string
  children?: MarkdownNode[]
  position?: {
    start?: { offset?: number }
    end?: { offset?: number }
  }
}

/**
 * CommonMark 把成对 `_` 当成斜体/加粗，把单独成行的 `___` 当成分隔线。
 * 助手回答里的普通下划线、文件名和标识符（`_name_`、`file_name`、`__value__`）
 * 因此会丢掉这些字符。星号强调和 `---` / `***` 分隔线仍是有意 Markdown，不动。
 * 输入框是纯文本，用户气泡也不走 Markdown，所以修复只放在解析之后。
 */
export function remarkLiteralUnderscore() {
  return (tree: MarkdownNode, file: { value?: unknown } | string) => {
    const source = typeof file === 'string' ? file : String(file?.value ?? '')
    if (!source || !tree.children) return
    tree.children = tree.children.flatMap((child) => expandLiteralUnderscore(child, source))
  }
}

function expandLiteralUnderscore(node: MarkdownNode, source: string): MarkdownNode[] {
  if (node.type === 'thematicBreak') {
    const raw = sourceSlice(node, source)
    const line = raw.replace(/\s+$/, '')
    if (/^[ \t]*(?:_[ \t]*){3,}$/.test(line)) {
      return [{ type: 'paragraph', children: [{ type: 'text', value: line.trim() }] }]
    }
    return [node]
  }

  if (node.children) {
    node.children = node.children.flatMap((child) => expandLiteralUnderscore(child, source))
  }

  if (node.type !== 'emphasis' && node.type !== 'strong') return [node]
  const start = node.position?.start?.offset
  const end = node.position?.end?.offset
  if (start == null || end == null || source[start] !== '_') return [node]

  const first = node.children?.[0]?.position?.start?.offset
  const last = node.children?.[node.children.length - 1]?.position?.end?.offset
  if (first == null || last == null || first < start || last > end) {
    return [{ type: 'text', value: source.slice(start, end) }]
  }
  return [
    { type: 'text', value: source.slice(start, first) },
    ...(node.children ?? []),
    { type: 'text', value: source.slice(last, end) },
  ]
}

function sourceSlice(node: MarkdownNode, source: string): string {
  const start = node.position?.start?.offset
  const end = node.position?.end?.offset
  if (start == null || end == null) return ''
  return source.slice(start, end)
}
