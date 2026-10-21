import { formatCitations, type CiteItem, type Reference, type Style } from './citations'
const PREFIX = 'kivio-cite:'
const BIB = 'kivio-bibliography'
const KEY = 'kivio-study-v1'
type Data = { version: 1; style: Style; references: Reference[]; clusters: Record<string, CiteItem[]> }
export async function updateDocument(references: Reference[], style: Style, action: 'insert' | 'refresh' | 'bibliography' | 'edit', selection: CiteItem[] = []) {
  const settings = Office.context.document.settings
  await new Promise<void>((resolve, reject) => settings.refreshAsync(r => r.status === Office.AsyncResultStatus.Succeeded ? resolve() : reject(new Error(r.error.message))))
  const saved = settings.get(KEY) as Data | null
  if (saved && saved.version !== 1) throw new Error('文档引用数据版本不支持，请更新 Kivio。')
  const data: Data = saved ?? { version: 1, style, references: [], clusters: {} }
  const available = new Map(data.references.map(r => [r.id, r]))
  references.forEach(r => available.set(r.id, r))
  data.style = style
  if ((action === 'insert' || action === 'edit') && selection.some(item => !available.has(item.id))) {
    throw new Error('所选文献已从 Study 移除，请重新选择后插入。')
  }
  await Word.run(async context => {
    const controls = context.document.body.contentControls
    controls.load('items/id,items/tag')
    const selected = context.document.getSelection().parentContentControlOrNullObject
    selected.load('tag,id')
    await context.sync()
    if (action === 'insert' || action === 'edit') {
      if (!selection.length) throw new Error('请先选择至少一篇文献。')
      if (action === 'insert' && !selected.isNullObject && selected.tag.startsWith('kivio-')) throw new Error('光标位于已有引用中。请使用“替换当前引用”，或将光标移到正文后插入。')
      if (action === 'edit' && (selected.isNullObject || !selected.tag.startsWith(PREFIX))) throw new Error('请先把 Word 光标放在要修改的 Kivio 引用中。')
      const tag = action === 'edit' ? selected.tag : `${PREFIX}${crypto.randomUUID()}`
      data.clusters[tag] = selection
      // Save references before inserting a control: a failed document write remains recoverable.
      const needed = new Set(Object.values(data.clusters).flat().map(item => item.id))
      data.references = Array.from(available.values()).filter(item => needed.has(item.id))
      settings.set(KEY, data)
      await saveSettings()
      if (action === 'insert') {
        const control = context.document.getSelection().getRange('End').insertContentControl()
        control.tag = tag
        control.title = 'Kivio Study 引用'
        control.insertText('更新引用…', 'Replace')
        await context.sync()
      }
    }
    controls.load('items/id,items/tag')
    await context.sync()
    const citations = controls.items.filter(c => c.tag.startsWith(PREFIX))
    const clusters = citations.map(c => {
      const items = data.clusters[c.tag]
      if (!items) throw new Error('发现无法读取的 Kivio 引用，请撤销粘贴或从文献库重新插入；原引用尚未改动。')
      return { citationID: String(c.id), citationItems: items, properties: { noteIndex: 0 } }
    })
    const formatted = formatCitations(Array.from(available.values()), clusters, style)
    const used = new Set(clusters.flatMap(c => c.citationItems.map(i => i.id)))
    data.references = Array.from(available.values()).filter(r => used.has(r.id))
    const tags = new Set(citations.map(c => c.tag))
    data.clusters = Object.fromEntries(Object.entries(data.clusters).filter(([tag]) => tags.has(tag)))
    settings.set(KEY, data)
    await saveSettings()
    for (const control of citations) control.insertHtml(formatted.citations.get(String(control.id)) ?? '', 'Replace')
    const bibliographies = controls.items.filter(c => c.tag === BIB)
    if (action === 'bibliography' && !bibliographies.length) {
      if (!selected.isNullObject && selected.tag.startsWith('kivio-')) throw new Error('请将光标移到正文空白处再插入参考文献。')
      const control = context.document.getSelection().getRange('End').insertContentControl()
      control.tag = BIB
      control.title = 'Kivio Study 参考文献'
      bibliographies.push(control)
    }
    for (const control of bibliographies) control.insertHtml(formatted.bibliography || '<p>尚无引用</p>', 'Replace')
    await context.sync()
  })
}
async function saveSettings() {
  await new Promise<void>((resolve, reject) => Office.context.document.settings.saveAsync(r => r.status === Office.AsyncResultStatus.Succeeded ? resolve() : reject(new Error(r.error.message))))
}
export function documentStyle(): Style {
  const data = Office.context.document.settings.get(KEY) as Data | null
  return data?.style === 'apa' || data?.style === 'ieee' ? data.style : 'gbt'
}
