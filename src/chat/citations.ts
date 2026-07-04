// 知識庫引用 `[n]` 的 mdast 切分邏輯 + remark 外掛。
// 抽成獨立模組（非元件），既便於單測，也避免 ChatMarkdown.tsx 觸發
// react-refresh/only-export-components。

// 極簡 mdast 節點檢視：只需 type/value/url/children 來切分文字節點。
export interface MdNode {
  type: string
  value?: string
  url?: string
  children?: MdNode[]
}

/** 把一個文字節點裡的 `[n]`（且 n 是有效引用）切成 text / link 混排。 */
export function splitCitations(value: string, validNs: Set<number>): MdNode[] {
  const out: MdNode[] = []
  let last = 0
  const re = /\[(\d{1,3})\]/g
  let m: RegExpExecArray | null
  while ((m = re.exec(value))) {
    const n = Number(m[1])
    if (!validNs.has(n)) continue
    if (m.index > last) out.push({ type: 'text', value: value.slice(last, m.index) })
    out.push({ type: 'link', url: `#kb-cite-${n}`, children: [{ type: 'text', value: `[${n}]` }] })
    last = m.index + m[0].length
  }
  if (last < value.length) out.push({ type: 'text', value: value.slice(last) })
  return out
}

/** remark 外掛：遍歷樹，把 text 節點裡的有效 `[n]` 換成 `#kb-cite-n` 連結。
 *  跳過 link/code，避免巢狀連結或汙染程式碼。 */
export function remarkCitations(validNs: Set<number>) {
  const walk = (node: MdNode) => {
    if (!node.children || node.type === 'link' || node.type === 'linkReference') return
    const next: MdNode[] = []
    for (const child of node.children) {
      if (child.type === 'text' && child.value && /\[\d{1,3}\]/.test(child.value)) {
        next.push(...splitCitations(child.value, validNs))
      } else {
        walk(child)
        next.push(child)
      }
    }
    node.children = next
  }
  return () => (tree: MdNode) => walk(tree)
}
