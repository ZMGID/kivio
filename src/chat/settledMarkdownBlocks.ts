import { isValidElement, type ReactNode } from 'react'

// Streamdown uses one fixed plugin and component set for chat. Links, citations,
// and artifacts read the current conversation from context when React mounts them.
const MAX_SOURCE_CHARS = 500_000
const MAX_NODES = 40_000
const ENTRY_OVERHEAD = 200
const blocks = new Map<string, { children: ReactNode; nodes: number }>()
let cachedChars = 0
let cachedNodes = 0

export function readSettledBlock(source: string) {
  const entry = blocks.get(source)
  if (entry) {
    blocks.delete(source)
    blocks.set(source, entry)
  }
  return entry?.children
}

export function rememberSettledBlock(source: string, children: ReactNode) {
  const size = source.length + ENTRY_OVERHEAD
  if (!children || size > MAX_SOURCE_CHARS / 4 || blocks.has(source)) return
  // A dense table can expand short source into thousands of elements.
  const pending: ReactNode[] = [children]
  let nodes = 0
  while (pending.length) {
    const node = pending.pop()
    if (Array.isArray(node)) {
      if (nodes + pending.length + node.length > MAX_NODES / 4) return
      for (const child of node) pending.push(child)
    } else {
      if (++nodes > MAX_NODES / 4) return
      if (isValidElement<{ children?: ReactNode }>(node) && node.props.children != null) {
        pending.push(node.props.children)
      }
    }
  }
  blocks.set(source, { children, nodes })
  cachedChars += size
  cachedNodes += nodes
  for (const [oldest, entry] of blocks) {
    if (cachedChars <= MAX_SOURCE_CHARS && cachedNodes <= MAX_NODES) break
    blocks.delete(oldest)
    cachedChars -= oldest.length + ENTRY_OVERHEAD
    cachedNodes -= entry.nodes
  }
}
