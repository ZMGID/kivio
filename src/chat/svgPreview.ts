const SVG_NAMESPACE = 'http://www.w3.org/2000/svg'

export type SvgSnapshot = { svg: string; complete: boolean; aspectRatio: number }
export type SvgPreviewResult = { kind: 'svg' | 'html'; snapshot: SvgSnapshot | null }

/** Only standalone SVG belongs in the image renderer, not an HTML page containing a chart. */
export function isSvgSource(source: string): boolean {
  return /^\s*(?:(?:<\?xml[\s\S]*?\?>|<!--[\s\S]*?-->)\s*)*<svg(?:\s|\/?>|$)/.test(source)
}

/** Keeps complete XML tokens, drops the unfinished tail and closes open elements for preview only. */
export function readSvgPreview(source: string): SvgPreviewResult {
  const stack: string[] = []
  let cursor = 0
  let safeEnd = 0
  let rootStart = -1
  let rootClosed = false

  while (cursor < source.length) {
    if (source.startsWith('<!--', cursor)) {
      const end = source.indexOf('-->', cursor + 4)
      if (end < 0) break
      cursor = end + 3
      safeEnd = cursor
      continue
    }
    if (source.startsWith('<?xml', cursor) && rootStart < 0) {
      const end = source.indexOf('?>', cursor + 5)
      if (end < 0) break
      cursor = end + 2
      safeEnd = cursor
      continue
    }
    if (source.startsWith('<![CDATA[', cursor) && stack.length > 0) {
      const end = source.indexOf(']]>', cursor + 9)
      if (end < 0) break
      cursor = end + 3
      safeEnd = cursor
      continue
    }
    // No DTD/entity expansion or processing instructions in model-generated images.
    if (source.startsWith('<!', cursor) || source.startsWith('<?', cursor)) return { kind: 'svg', snapshot: null }
    if (source[cursor] !== '<') {
      const next = source.indexOf('<', cursor)
      let end = next < 0 ? source.length : next
      const text = source.slice(cursor, end)
      if (stack.length === 0 && text.trim()) return { kind: 'html', snapshot: null }
      if (next < 0 && stack.length > 0) {
        // Markdown's code renderer can append newlines after the unfinished entity.
        const unfinishedEntity = /&[^;\s]*\s*$/.exec(text)
        if (unfinishedEntity) end = cursor + unfinishedEntity.index
      }
      safeEnd = end
      if (end !== (next < 0 ? source.length : next)) break
      cursor = end
      continue
    }
    if (rootClosed) return { kind: 'html', snapshot: null }

    // A '>' inside an attribute value is not a tag boundary.
    let end = cursor + 1
    let quote = ''
    for (; end < source.length; end++) {
      const char = source[end]
      if (quote) {
        if (char === quote) quote = ''
      } else if (char === '"' || char === "'") {
        quote = char
      } else if (char === '>') {
        break
      }
    }
    if (end === source.length) break
    const token = source.slice(cursor, end + 1)
    const closing = /^<\/([A-Za-z_][\w.:-]*)\s*>$/.exec(token)
    if (closing) {
      if (stack.pop() !== closing[1]) return { kind: 'svg', snapshot: null }
      rootClosed = stack.length === 0
    } else {
      const opening = /^<([A-Za-z_][\w.:-]*)(?:\s|\/?>)/.exec(token)
      if (!opening) return { kind: 'svg', snapshot: null }
      if (rootStart < 0) {
        if (opening[1] !== 'svg') return { kind: 'html', snapshot: null }
        rootStart = cursor
      }
      if (!/\/\s*>$/.test(token)) stack.push(opening[1])
      else if (stack.length === 0) rootClosed = true
    }
    cursor = end + 1
    safeEnd = cursor
  }

  if (rootStart < 0) return { kind: 'svg', snapshot: null }
  let svg = source.slice(0, safeEnd) + [...stack].reverse().map((name) => `</${name}>`).join('')
  const parser = new DOMParser()
  let document = parser.parseFromString(svg, 'image/svg+xml')
  if (document.querySelector('parsererror')) return { kind: 'svg', snapshot: null }
  // Inline SVG often omits xmlns; image/svg+xml needs the SVG namespace.
  if (!document.documentElement.namespaceURI) {
    svg = svg.slice(0, rootStart + 4) + ` xmlns="${SVG_NAMESPACE}"` + svg.slice(rootStart + 4)
    document = parser.parseFromString(svg, 'image/svg+xml')
  }
  const root = document.documentElement
  if (document.querySelector('parsererror') || root.namespaceURI !== SVG_NAMESPACE) return { kind: 'svg', snapshot: null }

  // The result is loaded as an image, never injected into the chat DOM. Strip active content too.
  for (const element of [root, ...root.querySelectorAll('*')]) {
    if (element.localName.toLowerCase() === 'script') {
      element.remove()
      continue
    }
    for (const attribute of [...element.attributes]) {
      if (/^on/i.test(attribute.localName)) element.removeAttributeNode(attribute)
    }
  }
  const viewBox = root.getAttribute('viewBox')?.trim().split(/[\s,]+/).map(Number)
  const width = root.getAttribute('width') ?? ''
  const height = root.getAttribute('height') ?? ''
  let aspectRatio = 16 / 9
  if (viewBox?.length === 4 && viewBox.every(Number.isFinite) && viewBox[2] > 0 && viewBox[3] > 0) {
    aspectRatio = viewBox[2] / viewBox[3]
  } else if (/^(?:\d+(?:\.\d*)?|\.\d+)(?:px)?$/.test(width) && /^(?:\d+(?:\.\d*)?|\.\d+)(?:px)?$/.test(height)
    && Number.parseFloat(width) > 0 && Number.parseFloat(height) > 0) {
    aspectRatio = Number.parseFloat(width) / Number.parseFloat(height)
  }
  return {
    kind: 'svg',
    snapshot: { svg: new XMLSerializer().serializeToString(root), complete: rootClosed && cursor === source.length, aspectRatio },
  }
}
