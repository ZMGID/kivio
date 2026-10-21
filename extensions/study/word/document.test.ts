import { afterEach, expect, it, vi } from 'vitest'
import { updateDocument } from './document'
import type { Reference } from './citations'

// External Office adapter only. Formatting and the document owner run unchanged.
function office(initial: unknown, controls: { id: number; tag: string; html: string }[], failSave = false) {
  let stored = structuredClone(initial)
  let draft = structuredClone(initial)
  const items = controls.map(c => ({ ...c, insertHtml(html: string) { this.html = html } }))
  vi.stubGlobal('Office', { AsyncResultStatus: { Succeeded: 'ok' }, context: { document: { settings: {
    refreshAsync: (done: (r: unknown) => void) => { draft = structuredClone(stored); done({ status: 'ok' }) },
    get: () => draft,
    set: (_key: string, data: unknown) => { draft = data },
    saveAsync: (done: (r: unknown) => void) => { if (failSave) done({ status: 'error', error: { message: 'Save failed' } }); else { stored = structuredClone(draft); done({ status: 'ok' }) } },
  } } } })
  vi.stubGlobal('Word', { run: async (action: (context: unknown) => Promise<void>) => action({
    document: { body: { contentControls: { items, load() {} } }, getSelection: () => ({ parentContentControlOrNullObject: { isNullObject: true, load() {} } }) }, sync: async () => {},
  }) })
  return { items, saved: () => stored }
}
const paper: Reference = { id: 'paper', name: 'Stored paper', bibliography: { itemType: 'article', authors: ['Smith, Ada'], year: '2024', publication: 'Journal', doi: '', url: '', abstractText: '' } }
const data = { version: 1, style: 'apa', references: [paper], clusters: { 'kivio-cite:one': [{ id: 'paper' }] } }
afterEach(() => vi.unstubAllGlobals())
it('refreshes a reopened document from its saved reference snapshot even after the library item is removed', async () => {
  const host = office(data, [{ id: 5, tag: 'kivio-cite:one', html: 'old' }, { id: 7, tag: 'kivio-bibliography', html: 'old' }])
  await updateDocument([], 'apa', 'refresh')
  expect(host.items[0].html).toBe('(Smith, 2024)')
  expect(host.items[1].html).toContain('Stored paper')
  expect(host.saved()).toMatchObject({ references: [{ id: 'paper' }] })
})
it('does not overwrite citation text when Word rejects saving its metadata', async () => {
  const host = office(data, [{ id: 5, tag: 'kivio-cite:one', html: 'original text' }], true)
  await expect(updateDocument([paper], 'ieee', 'refresh')).rejects.toThrow('Save failed')
  expect(host.items[0].html).toBe('original text')
})
it('retains original contents when a copied citation has no matching document record', async () => {
  const host = office(data, [{ id: 5, tag: 'kivio-cite:unknown', html: 'original text' }])
  await expect(updateDocument([paper], 'apa', 'refresh')).rejects.toThrow('无法读取')
  expect(host.items[0].html).toBe('original text')
})

it('rejects a removed selection before inserting a placeholder into Word', async () => {
  const host = office(data, [{ id: 5, tag: 'kivio-cite:one', html: 'original text' }])
  await expect(updateDocument([], 'apa', 'insert', [{ id: 'removed' }])).rejects.toThrow('已从 Study 移除')
  expect(host.items[0].html).toBe('original text')
  expect(host.saved()).toEqual(data)
})
