import { createHash } from 'node:crypto'
import { test, expect, type Page, type TestInfo } from '@playwright/test'

// Amano et al. (2023), The manifold costs of being a non-native English speaker
// in science, PLOS Biology. CC BY; credit the authors and DOI with evidence.
// Download the public original for this run; do not commit or bundle the PDF.
const SOURCE_PAGE = 'https://journals.plos.org/plosbiology/article?id=10.1371/journal.pbio.3002184'
const SOURCE_PDF = 'https://journals.plos.org/plosbiology/article/file?id=10.1371/journal.pbio.3002184&type=printable'
const SOURCE_SHA256 = '0012515e110c81d8fe0a739568c0c20c1521938595851e2b8acfe22be7ce2b8a'
const ABSTRACT_QUESTION = '请把摘要前两句译成自然的中文，并解释 impediment 在这里的意思。作者的主要判断是什么？'
const FIGURE_QUESTION = '请解释图 1 的 A 面板：横纵轴、颜色、线型和阴影分别表示什么？这能证明因果关系吗？'
const WHOLE_PAPER_QUESTION = '请总结整篇论文的全部方法、结果和局限。'
type Region = { x: number; y: number; width: number; height: number }
// Visually verified against 1020 × 1320 rendered pages, not OCR or extracted text.
// Abstract heading + complete paragraph. Figure 1 includes all panels, axes,
// legend, shared x-label, full caption and DOI, so interpretation has context.
const ABSTRACT_REGION: Region = { x: 320 / 1020, y: 655 / 1320, width: 647 / 1020, height: 402 / 1320 }
const FIGURE_REGION: Region = { x: 148 / 1020, y: 106 / 1320, width: 821 / 1020, height: 1102 / 1320 }
const PANEL_WITHOUT_LEGEND: Region = { x: 148 / 1020, y: 106 / 1320, width: 414 / 1020, height: 291 / 1320 }
let sourceBytes: Buffer

test.describe.configure({ timeout: 90_000 })

test.beforeAll(async ({ request }) => {
  const response = await request.get(SOURCE_PDF, { timeout: 45_000 })
  expect(response.ok(), `Official PLOS PDF unavailable: ${response.status()}`).toBeTruthy()
  sourceBytes = await response.body()
  expect(createHash('sha256').update(sourceBytes).digest('hex')).toBe(SOURCE_SHA256)
})

test.beforeEach(async ({ page }) => {
  // This height leaves the entire printed figure reachable for a real pointer drag.
  await page.setViewportSize({ width: 1366, height: 1100 })
  await page.goto('/tests/study-browser/index.html?lang=zh')
  await page.getByLabel('导入 PDF 或图片').setInputFiles({ name: 'Amano-et-al-2023.pdf', mimeType: 'application/pdf', buffer: sourceBytes })
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveAttribute('max', '27')
  await expect(page.locator('.kv-study-reader-paper canvas')).toBeVisible()
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', /^data:image\/png;base64,/)
  await expect(page.getByRole('button', { name: '帮助方式', exact: true })).toContainText('阅读问答')
  await expect(page.getByLabel('关于本页的问题')).toHaveAttribute('placeholder', '想了解这页的什么？也可以问图表或翻译英文')
  await expect(page.locator('#study-attempt')).toBeHidden()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeDisabled()
  await expect(page.locator('.kv-study-library')).toBeHidden()
  await expect(page.locator('.kv-study-reader-extracted')).toHaveCount(0)
  await expect(page.getByLabel('修正后的题目文字')).toHaveCount(0)
  await expect(page.getByRole('checkbox', { name: '同时发送页面 / 选区图片' })).toHaveCount(0)
})

async function jumpToPage(page: Page, number: number) {
  const input = page.getByRole('spinbutton', { name: '页码' })
  await input.fill(String(number))
  await input.press('Enter')
  await expect(input).toHaveValue(String(number))
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('alt', new RegExp(`第 ${number} 页`))
}

async function selectSourceCrop(page: Page, region: Region) {
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

async function originalImage(page: Page) {
  // Independently crop the visible source canvas, rather than calling the
  // production context-image function. Selection overlay tint is not source ink.
  const expected = await page.locator('.kv-study-reader-paper').evaluate(node => {
    const source = node.querySelector('canvas')!
    const selection = node.querySelector<HTMLElement>('.kv-study-reader-region')
    const region = selection ? {
      x: parseFloat(selection.style.left) / 100, y: parseFloat(selection.style.top) / 100,
      width: parseFloat(selection.style.width) / 100, height: parseFloat(selection.style.height) / 100,
    } : { x: 0, y: 0, width: 1, height: 1 }
    const width = source.width * region.width
    const height = source.height * region.height
    const scale = Math.min(1, 1600 / Math.max(width, height))
    const output = document.createElement('canvas')
    output.width = Math.max(1, Math.floor(width * scale)); output.height = Math.max(1, Math.floor(height * scale))
    output.getContext('2d')!.drawImage(source, source.width * region.x, source.height * region.y, width, height, 0, 0, output.width, output.height)
    return output.toDataURL('image/png')
  })
  expect(expected.length * 0.75).toBeLessThanOrEqual(1_500_000)
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', expected)
  return expected
}

async function readingRequest(page: Page, index: number, number: number, scope: 'page' | 'region', image: string) {
  const request = await page.evaluate(index => window.__studyTest.requests[index], index)
  expect(request.imageDataUrl).toBe(image)
  expect(request.model).toBe('test-vision')
  expect(request.systemPrompt).toContain('READ MODE')
  expect(request.systemPrompt).toContain("Answer the reader's question directly")
  expect(request.systemPrompt).toContain('No prior attempt is needed')
  expect(request.systemPrompt).toContain('Separate what the source visibly says from your explanation or inference')
  expect(request.systemPrompt).toContain('You cannot see other pages or the rest of the document')
  expect(request.systemPrompt).toContain('pageNumber is the actual document/PDF page index')
  expect(request.systemPrompt).not.toMatch(/HINT MODE|FULL SOLUTION MODE|RESPONSE FORMAT/)
  const context = JSON.parse(request.userPrompt.slice(request.userPrompt.indexOf('\n') + 1))
  expect(context.pageNumber).toBe(number)
  expect(context.sourceScope).toBe(scope)
  expect(context.sourceLabel).toBe(`Page ${number}${scope === 'region' ? ', selected region' : ''}`)
  expect(context.attempt).toBeNull()
  for (const key of ['text', 'pageText', 'recognizedText', 'extractedText', 'correctedText', 'pageTextTruncated']) expect(context).not.toHaveProperty(key)
  expect(context.context).toContain(scope === 'region' ? 'selected-region image only' : 'current-page image')
  return { request, context }
}

async function capture(page: Page, info: TestInfo, filename: string) {
  await page.locator('.kv-study-reader-scroll').evaluate(node => { node.scrollTop = 0 })
  await page.screenshot({ path: info.outputPath(filename), fullPage: true, animations: 'disabled' })
}
async function captureCrop(page: Page, info: TestInfo, filename: string) {
  const details = page.locator('.kv-study-reader-preview')
  if (!await details.evaluate(node => (node as HTMLDetailsElement).open)) await details.locator(':scope > summary').click()
  await expect(details.locator('img')).toBeVisible()
  const image = (await details.locator('img').getAttribute('src'))!
  // Attach all original crop pixels; a screenshot inside the 320px scrolling
  // preview would clip a tall figure and hide its caption from review.
  await info.attach(filename, { body: Buffer.from(image.split(',')[1], 'base64'), contentType: 'image/png' })
  await details.locator(':scope > summary').click()
}
async function send(page: Page, question: string, lesson: NonNullable<Window['__studyTest']['lesson']>) {
  await page.evaluate(value => { window.__studyTest.lesson = value }, lesson)
  await page.getByLabel('关于本页的问题').fill(question)
  await expect(page.locator('#study-attempt')).toBeHidden()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeEnabled()
  const requestCount = await page.evaluate(() => window.__studyTest.requests.length)
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.__studyTest.requests.length)).toBe(requestCount + 1)
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeEnabled()
  await expect(page.getByText('保存在此设备', { exact: true })).toBeVisible()
}
async function observations(info: TestInfo, details: Record<string, unknown>) {
  await info.attach('real-reading-observations.json', { body: JSON.stringify({
    authors: 'Amano et al.', year: 2023, source: SOURCE_PAGE, pdf: SOURCE_PDF, doi: '10.1371/journal.pbio.3002184', license: 'CC BY',
    sha256: SOURCE_SHA256, sourceBytes: sourceBytes.length, pageCount: 27,
    sourceContext: 'Exact original rendered page/crop PNG; no OCR, extraction, corrected source text, or document-wide index',
    teachingValidation: 'Conspicuously labelled fixed simulated replies; not live-model translation, reading or figure-comprehension validation',
    ...details,
  }, null, 2), contentType: 'application/json' })
}

// Rendering, pointer selection, source bytes, navigation, storage and prompt
// assembly are real. Fixed responses test presentation, never model quality.
test('English abstract translation uses default reading, exact images and honest page-only scope', async ({ page }, info) => {
  const fullPage = await originalImage(page)
  await capture(page, info, 'real-reading-chinese-fresh-desktop.png')
  await send(page, ABSTRACT_QUESTION, 'amano-abstract')
  await expect(page.locator('.kv-study-turn').last()).toContainText('演示阅读（非真实模型调用）')
  const wholePageCall = await readingRequest(page, 0, 1, 'page', fullPage)
  expect(wholePageCall.context.question).toBe(ABSTRACT_QUESTION)
  expect(wholePageCall.request.history).toHaveLength(0)
  expect(wholePageCall.request.systemPrompt).toContain('Keep translation separate from explanation')
  expect(wholePageCall.request.userPrompt).not.toContain('By surveying 908 researchers')
  await expect(page.getByRole('button', { name: '展开完整解答', exact: true })).toHaveCount(0)

  await selectSourceCrop(page, ABSTRACT_REGION)
  const abstractImage = await originalImage(page)
  expect(abstractImage).not.toBe(fullPage)
  await send(page, ABSTRACT_QUESTION, 'amano-abstract')
  const passage = await readingRequest(page, 1, 1, 'region', abstractImage)
  expect(passage.request.history).toHaveLength(2)
  await expect(page.locator('.kv-study-turn').last()).toContainText('impediment')
  await expect(page.locator('.kv-study-turn').last()).toContainText('908 名环境科学研究者')
  await expect(page.locator('.kv-study-turn').last()).toContainText('背景解释')
  await capture(page, info, 'real-reading-abstract-desktop.png')
  await captureCrop(page, info, 'real-reading-original-abstract-crop.png')

  await send(page, WHOLE_PAPER_QUESTION, 'amano-whole-paper')
  const limited = await readingRequest(page, 2, 1, 'region', abstractImage)
  expect(limited.request.systemPrompt).toContain('only that crop, not the full page')
  expect(limited.request.systemPrompt).toContain('not the whole paper')
  expect(limited.request.systemPrompt).toContain('Prior discussion is not independent evidence of unseen source content')
  await expect(page.locator('.kv-study-turn').last()).toContainText('不能据此总结整篇论文')
  // Changing page does not relabel historical source anchors or leak p1 history.
  await jumpToPage(page, 4)
  await page.getByLabel('关于本页的问题').fill('保留第 4 页的未发送草稿')
  await page.getByRole('button', { name: '展开材料栏', exact: true }).click()
  await page.locator('.kv-study-history-row').filter({ hasText: WHOLE_PAPER_QUESTION }).click()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('1')
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', abstractImage)
  await page.locator('.kv-study-turn').first().getByRole('button', { name: '来源：第 1 页', exact: true }).click()
  await expect(page.locator('.kv-study-reader-region')).toHaveCount(0)
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', fullPage)
  await page.locator('.kv-study-turn').last().getByRole('button', { name: '来源：第 1 页 · 选区', exact: true }).click()
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', abstractImage)
  await expect(page.getByText('保存在此设备', { exact: true })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('1')
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', abstractImage)
  await expect(page.getByLabel('关于本页的问题')).toHaveValue(WHOLE_PAPER_QUESTION)
  await expect(page.locator('.kv-study-turn')).toHaveCount(3)
  await jumpToPage(page, 4)
  await expect(page.getByLabel('关于本页的问题')).toHaveValue('保留第 4 页的未发送草稿')
  await expect(page.locator('.kv-study-turn')).toHaveCount(0)
  await observations(info, { journey: 'English abstract and whole-paper scope limit', page: 1, region: ABSTRACT_REGION, wholePageAndCropMatchOriginalPixels: true, noAttemptRequired: true, historySourceAndDraftRestored: true })
})

test('paper figure keeps axes, legend and caption, restores its original source, and carries mobile context', async ({ page }, info) => {
  await jumpToPage(page, 4)
  await selectSourceCrop(page, FIGURE_REGION)
  const figureImage = await originalImage(page)
  await capture(page, info, 'real-reading-figure-selection-desktop.png')
  await captureCrop(page, info, 'real-reading-original-figure-crop.png')
  await send(page, FIGURE_QUESTION, 'amano-figure')
  const figure = await readingRequest(page, 0, 4, 'region', figureImage)
  expect(figure.request.history).toHaveLength(0)
  expect(figure.request.systemPrompt).toContain('Do not infer causation or statistical significance from a chart or correlation alone')
  await expect(page.locator('.kv-study-turn').last()).toContainText('演示读图（非真实模型调用）')
  await expect(page.locator('.kv-study-turn').last()).toContainText('95% 置信区间')
  await expect(page.locator('.kv-study-turn').last()).toContainText('原图信息')
  await expect(page.locator('.kv-study-turn').last()).toContainText('不能证明因果')
  await capture(page, info, 'real-reading-figure-answer-desktop.png')

  // A different crop omits the legend/caption. The contract must ask for missing
  // evidence; the fixture limitation is explicit rather than a claimed model test.
  await selectSourceCrop(page, PANEL_WITHOUT_LEGEND)
  const incompleteImage = await originalImage(page)
  expect(incompleteImage).not.toBe(figureImage)
  await send(page, '只看当前选区，这几条彩色线和阴影各代表什么？', 'amano-missing-legend')
  const incomplete = await readingRequest(page, 1, 4, 'region', incompleteImage)
  expect(incomplete.request.systemPrompt).toContain('ask for a clearer or larger image that includes it')
  await expect(page.locator('.kv-study-turn').last()).toContainText('缺少图例和完整图注')
  await page.locator('.kv-study-turn').first().getByRole('button', { name: '来源：第 4 页 · 选区', exact: true }).click()
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', figureImage)
  await page.locator('.kv-study-notes > summary').click()
  await page.getByLabel('本页笔记', { exact: true }).fill('图 1：先核对轴、图例和图注；区分作者图示与自己的推断。')
  await jumpToPage(page, 1)
  await page.getByLabel('关于本页的问题').fill('摘要页的未发送问题')
  await page.getByRole('button', { name: '展开材料栏', exact: true }).click()
  await page.locator('.kv-study-history-row').filter({ hasText: FIGURE_QUESTION }).click()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('4')
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', figureImage)
  await expect(page.getByText('保存在此设备', { exact: true })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('4')
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', figureImage)
  await expect(page.locator('.kv-study-turn')).toHaveCount(2)
  await page.locator('.kv-study-notes > summary').click()
  await expect(page.getByLabel('本页笔记', { exact: true })).toHaveValue('图 1：先核对轴、图例和图注；区分作者图示与自己的推断。')
  await page.locator('.kv-study-notes > summary').click()

  await page.setViewportSize({ width: 390, height: 844 })
  await page.getByRole('tab', { name: '阅读', exact: true }).click()
  await page.locator('.kv-study-reader-scroll').evaluate(node => { node.scrollTop = 0 })
  await expect(page.getByRole('button', { name: '问这个区域', exact: true })).toBeInViewport()
  await capture(page, info, 'real-reading-mobile-figure-source-light.png')
  await page.getByRole('button', { name: '问这个区域', exact: true }).click()
  await expect(page.getByLabel('关于本页的问题')).toBeFocused()
  const anchor = page.getByRole('button', { name: '查看第 4 页选区', exact: true })
  await expect(anchor.locator('img')).toHaveAttribute('src', figureImage)
  await expect(page.locator('#study-attempt')).toBeHidden()
  expect(await page.evaluate(() => window.__studyTest.requests)).toHaveLength(0)
  await capture(page, info, 'real-reading-mobile-figure-help-light.png')
  await page.getByRole('button', { name: 'Toggle theme' }).click()
  await expect(page.locator('html')).toHaveClass(/dark/)
  await capture(page, info, 'real-reading-mobile-figure-help-dark.png')
  await anchor.click()
  await expect(page.getByRole('tab', { name: '阅读', exact: true })).toHaveAttribute('aria-selected', 'true')
  await expect(page.locator('.kv-study-reader-paper')).toBeFocused()
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', figureImage)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy()
  await observations(info, { journey: 'Figure 1 with source navigation and missing-caption limitation', page: 4, region: FIGURE_REGION, missingLegendRegion: PANEL_WITHOUT_LEGEND, completeFigureIncludesAxesLegendCaption: true, originalSourceImageRestored: true, mobileAskDoesNotSend: true, notesAndHistoryRestored: true })
})
