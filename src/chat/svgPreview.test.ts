// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { isSvgSource, readSvgPreview } from './svgPreview'

const root = '<svg viewBox="0 0 400 200">'
const rect = '<rect width="400" height="200" fill="blue"/>'

function snapshot(source: string) {
  const result = readSvgPreview(source)
  expect(result.kind).toBe('svg')
  if (result.kind !== 'svg' || !result.snapshot) throw new Error('Expected a drawable SVG snapshot')
  const document = new DOMParser().parseFromString(result.snapshot.svg, 'image/svg+xml')
  expect(document.querySelector('parsererror')).toBeNull()
  return { ...result.snapshot, document }
}

describe('progressive SVG snapshots', () => {
  it('keeps finished shapes when the next attribute is split, including quoted angle brackets', () => {
    const partial = snapshot(root + rect + '<circle aria-label="a > b" cx="')
    expect(partial.document.querySelector('rect')?.getAttribute('fill')).toBe('blue')
    expect(partial.document.querySelector('circle')).toBeNull()
    expect(partial.complete).toBe(false)
    const next = snapshot(root + rect + '<circle aria-label="a > b" cx="80" cy="80" r="30"/>')
    expect(next.document.querySelector('circle')?.getAttribute('aria-label')).toBe('a > b')
    expect(next.aspectRatio).toBe(2)
  })

  it('temporarily closes nested groups and text without changing the original geometry', () => {
    const partial = snapshot(root + '<g transform="translate(10 20)"><text x="15">Hello &am')
    expect(partial.document.querySelector('g')?.getAttribute('transform')).toBe('translate(10 20)')
    expect(partial.document.querySelector('text')?.textContent).toBe('Hello ')
    expect(partial.complete).toBe(false)
    const complete = snapshot(root + '<g transform="translate(10 20)"><text x="15">Hello &amp; world</text></g></svg>')
    expect(complete.document.querySelector('text')?.textContent).toBe('Hello & world')
    expect(complete.complete).toBe(true)
  })

  it('renders an unfinished entity even when Markdown appends trailing newlines', () => {
    const partial = snapshot(root + rect + '<text x="20">半成品 &am\n\n')
    expect(partial.document.querySelector('rect')?.getAttribute('fill')).toBe('blue')
    expect(partial.document.querySelector('text')?.textContent).toBe('半成品 ')
    expect(partial.complete).toBe(false)
    const resumed = snapshot(root + rect + '<text x="20">半成品 &amp;\n下一行</text></svg>')
    expect(resumed.document.querySelector('text')?.textContent).toBe('半成品 &\n下一行')
    expect(resumed.complete).toBe(true)
  })

  it('ignores unfinished comments and CDATA rather than turning them into visible text', () => {
    const comment = snapshot(root + rect + '<!-- unfinished >')
    expect(comment.document.documentElement.textContent).toBe('')
    const cdata = snapshot(root + rect + '<text><![CDATA[<unfinished')
    expect(cdata.document.querySelector('text')?.textContent).toBe('')
    const complete = snapshot(root + '<text><![CDATA[a < b]]></text></svg>')
    expect(complete.document.querySelector('text')?.textContent).toBe('a < b')
  })

  it('preserves gradients, local references and namespaces', () => {
    const image = snapshot('<?xml version="1.0"?>\n' + root + '<defs><linearGradient id="g"><stop offset="0" stop-color="red"/></linearGradient></defs><rect fill="url(#g)"/></svg>')
    expect(image.document.documentElement.namespaceURI).toBe('http://www.w3.org/2000/svg')
    expect(image.document.querySelector('linearGradient')?.id).toBe('g')
    expect(image.document.querySelector('rect')?.getAttribute('fill')).toBe('url(#g)')
    expect(image.complete).toBe(true)
  })

  it('does not treat ordinary HTML or SVG followed by page content as a pure SVG', () => {
    expect(isSvgSource('<html><body><svg></svg></body></html>')).toBe(false)
    expect(isSvgSource('<!-- chart -->\n<svg viewBox="0 0 1 1">')).toBe(true)
    expect(readSvgPreview(root + rect + '</svg><div>page content</div>').kind).toBe('html')
    expect(snapshot(root + '<svg><circle r="2"/></svg>' + rect + '</svg>').complete).toBe(true)
  })

  it('rejects mismatched closing tags, invalid entities and document type declarations', () => {
    for (const source of [root + '<g></svg>', root + '<text>&unknown;</text></svg>', '<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///etc/passwd">]>' + root + '</svg>']) {
      const result = readSvgPreview(source)
      expect(result.kind === 'svg' ? result.snapshot : null).toBeNull()
    }
  })

  it('does not allow scripts or event attributes in the image snapshot', () => {
    const image = snapshot(root + '<script>alert(1)</script><rect onload="alert(2)" width="10"/><foreignObject width="10" height="10"><div xmlns="http://www.w3.org/1999/xhtml" onclick="alert(3)">label</div></foreignObject></svg>')
    expect(image.document.querySelector('script')).toBeNull()
    expect(image.document.querySelector('rect')?.hasAttribute('onload')).toBe(false)
    expect(image.document.querySelector('div')?.hasAttribute('onclick')).toBe(false)
    expect(image.document.querySelector('div')?.textContent).toBe('label')
  })

  it('uses explicit dimensions when there is no viewBox and rejects an unfinished root', () => {
    expect(snapshot('<svg width="600px" height="300px"><circle r="10"/></svg>').aspectRatio).toBe(2)
    expect(readSvgPreview('<svg viewBox="0 0').snapshot).toBeNull()
  })
})
