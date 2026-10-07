import { createHash } from 'node:crypto'
import { test, expect, type Locator, type Page, type TestInfo } from '@playwright/test'
import type { Conversation } from '../../src/chat/types'
import type { FixtureLesson } from './mockTransport'
export type Region = { x: number; y: number; width: number; height: number }
export const editor = (page: Page) => page.locator('[data-composer-presentation="reading"] .chat-composer-editor[role="textbox"]')
export const composer = (page: Page) => page.locator('[data-composer-presentation="reading"]')
export const messages = (page: Page) => page.locator('[data-message-presentation="reading"]')
export const sendButton = (page: Page) => composer(page).locator('button.chat-composer-send')
export const attempt = (page: Page) => page.locator('.kv-study-reading-controls textarea')

export async function openStudy(page: Page, query = '') {
  await page.goto(`/tests/study-browser/index.html${query}#chat/study`)
  await expect(page.locator('.chat-window-shell')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Kivio Study', exact: true })).toBeVisible()
}
export async function expectSharedChat(page: Page) {
  await expect(editor(page)).toBeVisible()
  await expect(page.locator('[data-chat-composer="true"]')).toHaveCount(1)
  await expect(messages(page)).toHaveCount(1)
  await expect(page.locator('.kv-study-composer, .kv-study-turn')).toHaveCount(0)
}
async function selectedMaterial(page: Page) {
  return page.evaluate(() => new Promise<{ id: string | null; page: number | null; name: string | null }>((resolve, reject) => {
    const open = indexedDB.open('kivio-study')
    open.onerror = () => reject(open.error)
    open.onsuccess = () => {
      const db = open.result
      const tx = db.transaction(['meta', 'documents'], 'readonly')
      tx.onerror = () => { db.close(); reject(tx.error) }
      const workspace = tx.objectStore('meta').get('workspace')
      workspace.onsuccess = () => {
        const id = workspace.result?.selectedDocumentId as string | null | undefined
        if (!id) { db.close(); resolve({ id: null, page: null, name: null }); return }
        const document = tx.objectStore('documents').get(id)
        document.onsuccess = () => { db.close(); resolve({ id, page: document.result?.document.lastPage ?? null, name: document.result?.document.name ?? null }) }
      }
    }
  }))
}
export async function importMaterial(page: Page, file: { name: string; mimeType: string; buffer: Buffer }) {
  const id = createHash('sha256').update(file.buffer).digest('hex')
  const duplicate = (await savedConversations(page)).some(item => item.study_context?.materialId === id)
  const importButton = page.getByRole('button', { name: '导入材料', exact: true })
  // setInputFiles can bypass the visible import button's disabled state. Respect
  // the actual UI gate so a pending duplicate import cannot swallow the next file.
  await expect(importButton).toBeEnabled()
  await page.getByLabel('导入 PDF 或图片').setInputFiles(file)
  if (duplicate) await expect(page.getByText(/This material is already in your library/)).toBeVisible()
  await expect(importButton).toBeEnabled()
  await expect.poll(async () => (await selectedMaterial(page)).id, { message: 'The imported file hash must be selected durably' }).toBe(id)
  const selected = await selectedMaterial(page)
  const conversationId = `conv_study_${id}_${selected.page}`
  await expect.poll(() => page.evaluate(id => window.__studyTest.openedConversations.includes(id), conversationId), { message: 'The normal Chat owner must load the imported material conversation' }).toBe(true)
  await expect(page.locator('.kv-study-reader-title h2')).toHaveText(selected.name!)
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', /^data:image\/png;base64,/)
  await expectSharedChat(page)
  return id
}

export async function jumpToPage(page: Page, number: number) {
  const input = page.getByRole('spinbutton', { name: '页码' })
  await input.fill(String(number)); await input.press('Enter')
  await expect(input).toHaveValue(String(number))
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('alt', new RegExp(`第 ${number} 页`))
  await expectSharedChat(page)
}
export async function selectMode(page: Page, name: string) {
  await page.getByRole('button', { name: '帮助方式', exact: true }).click()
  await page.getByRole('option', { name, exact: true }).click()
}
export async function savedConversations(page: Page): Promise<Conversation[]> {
  return page.evaluate(() => JSON.parse(localStorage.getItem('kivio-chat-dev-conversations') ?? '[]'))
}
export async function send(page: Page, question: string, lesson?: FixtureLesson) {
  await page.evaluate(value => { window.__studyTest.lesson = value }, lesson)
  const count = await page.evaluate(() => window.__studyTest.requests.length)
  await editor(page).fill(question)
  await expect(sendButton(page)).toBeEnabled()
  await sendButton(page).click()
  await expect.poll(() => page.evaluate(() => window.__studyTest.requests.length)).toBe(count + 1)
  const call = await page.evaluate(index => window.__studyTest.requests[index], count)
  await expect.poll(async () => (await savedConversations(page)).find(item => item.id === call.conversationId)?.messages.at(-1)?.stream_outcome).toBe('completed')
  await expect(editor(page)).toHaveText('')
  await expectSharedChatSettled(page)
  return call
}
export async function sentSource(page: Page, index: number, pageNumber: number, mode: string, image: string) {
  const call = await page.evaluate(index => window.__studyTest.requests[index], index)
  expect(call.model).toMatch(/^test-vision/)
  expect(call.studySource?.page).toBe(pageNumber)
  expect(call.studySource?.mode).toBe(mode)
  expect(Object.keys(call.studySource ?? {}).sort()).toEqual(['attempt', 'mode', 'page', 'region'])
  expect(call.attachments).toHaveLength(1)
  expect(call.attachments[0].type).toBe('image')
  const actual = await page.evaluate(path => window.__studyTest.readImage(path), call.attachments[0].path)
  expect(actual === image, `Chat image attachment differs: actual ${imageHash(actual)}, expected ${imageHash(image)}`).toBe(true)
  expect(JSON.stringify(call)).not.toMatch(/pageText|extractedText|correctedText|recognizedText/)
  const conversation = (await savedConversations(page)).find(item => item.id === call.conversationId)!
  expect(conversation.study_context?.page).toBe(pageNumber)
  const user = conversation.messages.filter(message => message.role === 'user').at(-1)!
  expect(user.study_source).toEqual(call.studySource)
  expect(user.attachments?.[0].path).toBe(call.attachments[0].path)
  return call
}
export async function expectSharedChatSettled(page: Page) {
  await expect(page.getByRole('button', { name: /^(停止生成|正在停止|Stop generating|Stopping)$/ })).toHaveCount(0)
  await expect(page.getByRole('status', { name: '正在加载对话', exact: true })).toBeHidden()
  await expect(page.locator('[data-chat-message-list-item="streaming"], [data-chat-message-list-item="live-group"]')).toHaveCount(0)
}
export async function capture(page: Page, info: TestInfo, filename: string) {
  // A persisted terminal can precede the shared preview twin being reconciled.
  // Wait for that normal UI lifecycle, not a sleep or a hidden status graphic.
  await expectSharedChatSettled(page)
  await info.attach(`${filename}.state.json`, { body: JSON.stringify({
    visibleCancelControls: await page.getByRole('button', { name: /^(停止生成|正在停止|Stop generating|Stopping)$/ }).count(),
    liveRows: await page.locator('[data-chat-message-list-item="streaming"], [data-chat-message-list-item="live-group"]').count(),
    idlePresenceMarkers: await page.locator('.kv-stream-status-idle').count(),
    renderedMessageIds: await page.locator('[data-message-presentation="reading"] [data-message-id]').evaluateAll(nodes => nodes.map(node => node.getAttribute('data-message-id'))),
  }, null, 2), contentType: 'application/json' })
  await page.screenshot({ path: info.outputPath(filename), fullPage: true, animations: 'disabled' })
}
export async function captureCrop(page: Page, info: TestInfo, filename: string) {
  const image = (await page.locator('.kv-study-reader-preview img').getAttribute('src'))!
  await info.attach(filename, { body: Buffer.from(image.split(',')[1], 'base64'), contentType: 'image/png' })
}
export function pdf(pages: string[]) {
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', `<< /Type /Pages /Kids [${pages.map((_, i) => `${4 + i * 2} 0 R`).join(' ')}] /Count ${pages.length} >>`, '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>']
  pages.forEach((text, i) => {
    const stream = `BT /F1 18 Tf 48 740 Td (${text.replace(/[()\\]/g, '\\$&')}) Tj ET`
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + i * 2} 0 R >>`, `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`)
  })
  let result = '%PDF-1.4\n'; const offsets = [0]
  objects.forEach((object, i) => { offsets.push(Buffer.byteLength(result)); result += `${i + 1} 0 obj\n${object}\nendobj\n` })
  const xref = Buffer.byteLength(result)
  result += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.slice(1).map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`
  return Buffer.from(result)
}
export const material = (name = 'Reading handout.pdf', pages = ['An English passage to understand.', 'A second page about variables.', 'A third page about figures.']) => ({ name, mimeType: 'application/pdf', buffer: pdf(pages) })

export async function selectSourceCrop(page: Page, region: Region) {
  await page.getByRole('button', { name: '框选', exact: true }).click()
  await page.locator('.kv-study-reader-scroll').evaluate(node => { node.scrollTop = 0 })
  const paper = page.locator('.kv-study-reader-paper')
  const box = (await paper.boundingBox())!
  const scrollBox = (await page.locator('.kv-study-reader-scroll').boundingBox())!
  const start = { x: box.x + box.width * region.x, y: box.y + box.height * region.y }
  const end = { x: box.x + box.width * (region.x + region.width), y: box.y + box.height * (region.y + region.height) }
  expect(start.y).toBeGreaterThan(scrollBox.y)
  expect(end.y).toBeLessThan(scrollBox.y + scrollBox.height)
  await page.mouse.move(start.x, start.y)
  await page.mouse.down()
  expect((await paper.boundingBox())!.y).toBeCloseTo(box.y, 0)
  await page.mouse.move(end.x, end.y, { steps: 12 })
  await page.mouse.up()
  await expect(page.locator('.kv-study-reader-region')).toBeVisible()
  const actual = await page.locator('.kv-study-reader-region').evaluate(node => {
    const style = (node as HTMLElement).style
    return { x: parseFloat(style.left) / 100, y: parseFloat(style.top) / 100, width: parseFloat(style.width) / 100, height: parseFloat(style.height) / 100 }
  })
  for (const key of ['x', 'y', 'width', 'height'] as const) expect(actual[key]).toBeCloseTo(region[key], 2)
  await expect(page.locator('.kv-study-reader-preview')).not.toHaveAttribute('open', '')
}

function imageHash(image: string | null | undefined) {
  return image ? createHash('sha256').update(Buffer.from(image.split(',')[1], 'base64')).digest('hex') : null
}

export async function expectImageSource(locator: Locator, expected: string) {
  // Compare the exact string without putting a megabyte-long expected value in
  // Playwright's retried assertion log and every trace snapshot.
  await expect.poll(async () => {
    const actual = await locator.getAttribute('src')
    return { exactMatch: actual === expected, sha256: imageHash(actual) }
  }, { message: 'The restored source PNG must match the original bytes exactly' }).toEqual({ exactMatch: true, sha256: imageHash(expected) })
}

export async function originalImage(page: Page, materialId: string) {
  await expect(page.getByText('材料草稿保存在此设备', { exact: true })).toBeVisible()
  const saved = await page.evaluate(id => new Promise<{ page: number; region: Region | null }>((resolve, reject) => {
    const open = indexedDB.open('kivio-study')
    open.onerror = () => reject(open.error)
    open.onsuccess = () => {
      const db = open.result
      const request = db.transaction('documents', 'readonly').objectStore('documents').get(id)
      request.onerror = () => { db.close(); reject(request.error) }
      request.onsuccess = () => {
        db.close()
        const document = request.result?.document as { lastPage: number; pages: Record<string, { region?: Region | null }> } | undefined
        if (!document) { reject(new Error('The imported source must be saved before comparing its crop.')); return }
        resolve({ page: document.lastPage, region: document.pages[String(document.lastPage)]?.region ?? null })
      }
    }
  }), materialId)
  expect(saved.page).toBe(Number(await page.getByRole('spinbutton', { name: '页码' }).inputValue()))
  expect(Boolean(saved.region)).toBe(await page.locator('.kv-study-reader-region').count() > 0)
  // Independently crop the visible canvas using the exact saved drag coordinates.
  // CSS serializes percentages to limited precision (e.g. 31.3725%), which can
  // change drawImage interpolation even when the crop dimensions are unchanged.
  // Never call the production context-image function or use the tinted overlay.
  const expected = await page.locator('.kv-study-reader-paper').evaluate((node, storedRegion) => {
    const source = node.querySelector('canvas')!
    const region = storedRegion ?? { x: 0, y: 0, width: 1, height: 1 }
    const width = source.width * region.width
    const height = source.height * region.height
    const scale = Math.min(1, 1600 / Math.max(width, height))
    const output = document.createElement('canvas')
    output.width = Math.max(1, Math.floor(width * scale)); output.height = Math.max(1, Math.floor(height * scale))
    output.getContext('2d')!.drawImage(source, source.width * region.x, source.height * region.y, width, height, 0, 0, output.width, output.height)
    return {
      image: output.toDataURL('image/png'), width: output.width, height: output.height,
      sourceWidth: source.width, sourceHeight: source.height,
      overlayStyle: node.querySelector<HTMLElement>('.kv-study-reader-region')?.getAttribute('style') ?? null,
    }
  }, saved.region)
  const actual = await page.locator('.kv-study-reader-preview img').evaluate(async node => {
    const image = node as HTMLImageElement
    await image.decode()
    return { image: image.src, width: image.naturalWidth, height: image.naturalHeight }
  })
  const diagnostics = {
    page: saved.page, exactSavedRegion: saved.region, serializedOverlayStyle: expected.overlayStyle,
    sourceSize: [expected.sourceWidth, expected.sourceHeight],
    expectedSize: [expected.width, expected.height], actualSize: [actual.width, actual.height],
    expectedSha256: imageHash(expected.image), actualSha256: imageHash(actual.image),
  }
  await test.info().attach(`source-crop-oracle-page-${saved.page}.json`, { body: JSON.stringify(diagnostics, null, 2), contentType: 'application/json' })
  expect(expected.image.length * 0.75).toBeLessThanOrEqual(1_500_000)
  expect(actual.width).toBe(expected.width)
  expect(actual.height).toBe(expected.height)
  expect(actual.image === expected.image, JSON.stringify(diagnostics)).toBe(true)
  return expected.image
}


/** A raster-only PDF with no fonts or text-content stream. */
export function scannedPdf(jpeg: Buffer) {
  const content = 'q 612 0 0 792 0 0 cm /Scan Do Q'
  const objects = [
    Buffer.from('<< /Type /Catalog /Pages 2 0 R >>'),
    Buffer.from('<< /Type /Pages /Kids [3 0 R] /Count 1 >>'),
    Buffer.from('<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Scan 5 0 R >> >> /Contents 4 0 R >>'),
    Buffer.from(`<< /Length ${content.length} >>\nstream\n${content}\nendstream`),
    Buffer.concat([Buffer.from(`<< /Type /XObject /Subtype /Image /Width 720 /Height 900 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length ${jpeg.length} >>\nstream\n`), jpeg, Buffer.from('\nendstream')]),
  ]
  const chunks = [Buffer.from('%PDF-1.4\n')], offsets: number[] = []
  let length = chunks[0].length
  objects.forEach((object, index) => { offsets.push(length); const chunk = Buffer.concat([Buffer.from(`${index + 1} 0 obj\n`), object, Buffer.from('\nendobj\n')]); chunks.push(chunk); length += chunk.length })
  chunks.push(Buffer.from(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${length}\n%%EOF`))
  return Buffer.concat(chunks)
}
