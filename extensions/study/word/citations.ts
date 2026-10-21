import { Engine } from 'citeproc'
import apa from './styles/apa.csl?raw'
import gbt from './styles/gbt.csl?raw'
import ieee from './styles/ieee.csl?raw'
import en from './styles/en-US.xml?raw'
import zh from './styles/zh-CN.xml?raw'
import type { Bibliography } from '../api/reading'

export type Reference = { id: string; name: string; bibliography: Bibliography }
export type CiteItem = { id: string; locator?: string; label?: 'page' }
export type Cluster = { citationID: string; citationItems: CiteItem[]; properties: { noteIndex: number } }
export const styles = { gbt: { label: 'GB/T 7714—2015（顺序编码）', xml: gbt }, apa: { label: 'APA 7', xml: apa }, ieee: { label: 'IEEE', xml: ieee } }
export type Style = keyof typeof styles
export function formatCitations(references: Reference[], clusters: Cluster[], style: Style) {
  const items = new Map(references.map(({ id, name, bibliography: b }) => [id, {
    id, title: name,
    type: ({ article: 'article-journal', conference: 'paper-conference', book: 'book', thesis: 'thesis', webpage: 'webpage', preprint: 'article' } as Record<string, string>)[b.itemType] ?? 'document',
    author: b.authors.filter(a => a.trim()).map(a => {
      const comma = a.indexOf(',')
      return comma < 0 ? { literal: a.trim() } : { family: a.slice(0, comma).trim(), given: a.slice(comma + 1).trim() }
    }),
    ...(b.year && /^\d{4}$/.test(b.year) ? { issued: { 'date-parts': [[Number(b.year)]] } } : {}),
    'container-title': b.publication, DOI: b.doi, URL: b.url,
    volume: b.volume, issue: b.issue, page: b.pages, publisher: b.publisher, ISBN: b.isbn,
  }]))
  for (const c of clusters) for (const item of c.citationItems) if (!items.has(item.id)) throw new Error('引用资料缺失，请重新选择该文献。')
  const engine = new Engine({ retrieveLocale: lang => lang.startsWith('zh') ? zh : en, retrieveItem: id => items.get(id) }, styles[style].xml, style === 'gbt' ? 'zh-CN' : 'en-US')
  engine.setOutputFormat('html')
  const citations = engine.rebuildProcessorState(clusters, 'html')
  const bibliography = engine.makeBibliography()
  return { citations: new Map(citations.map(([id, , html]) => [id, html])), bibliography: bibliography ? bibliography[1].join('') : '' }
}
