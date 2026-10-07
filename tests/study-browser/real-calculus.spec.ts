import { createHash } from 'node:crypto'
import { test, expect, type Page } from '@playwright/test'

// Public source is fetched for this acceptance run, never checked in or bundled.
// MIT OCW / Arthur Mattuck, 18.01SC, Integration Techniques, exercise 5B-13.
// Source terms: https://ocw.mit.edu/pages/privacy-and-terms-of-use/
const SOURCE_PAGE = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/resources/mit18_01sc_pset5prb/'
const SOURCE_PDF = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/50d9ff5b7a30fe96bd69017ca5104d6e_MIT18_01SC_pset5prb.pdf'
const SOURCE_SHA256 = '5015876951651a4f4695ccbbb74e22fdf1a43b1798d4859ed3324cdc9e90d23d'
const SOLUTIONS = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/06979381db650b91c0de9d6755f03154_MIT18_01SC_pset5sol.pdf'
const ATTEMPT = '令 u=x^3\ndu=3x^2 dx\n原积分 = ∫du/(1+u^2)\n= arctan(u)+C = arctan(x^3)+C'

async function jumpToPage(page: Page, number: number) {
  await page.getByRole('spinbutton', { name: '页码' }).fill(String(number))
  await page.getByRole('spinbutton', { name: '页码' }).press('Enter')
  await expect(page.locator('.kv-study-page-label')).toHaveText(`第 ${number} 页`)
  await expect(page.locator('.kv-study-reader-paper canvas')).toBeVisible()
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('alt', new RegExp(`第 ${number} 页`))
}

async function selectSourceCrop(page: Page, region: { x: number; y: number; width: number; height: number }) {
  // Scroll the bottom source control into view before selecting a printed row.
  // Pointer focus must not move the tall PDF canvas and shift crop coordinates.
  await page.locator('.kv-study-reader-preview > summary').scrollIntoViewIfNeeded()
  await page.getByRole('button', { name: '框选', exact: true }).click()
  const paper = page.locator('.kv-study-reader-paper')
  const rect = (await paper.boundingBox())!
  await page.mouse.move(rect.x + rect.width * region.x, rect.y + rect.height * region.y)
  await page.mouse.down()
  const afterFocus = (await paper.boundingBox())!
  expect(afterFocus.y).toBeCloseTo(rect.y, 0)
  await page.mouse.move(rect.x + rect.width * (region.x + region.width), rect.y + rect.height * (region.y + region.height), { steps: 12 })
  await page.mouse.up()
  await expect(page.locator('.kv-study-reader-region')).toBeVisible()
  await expect(page.locator('.kv-study-reader-preview')).toHaveAttribute('open', '')
  await expect(page.locator('.kv-study-reader-extracted')).toHaveCount(0)
  await expect(page.getByLabel('修正后的题目文字')).toHaveCount(0)
  await expect(page.getByRole('checkbox', { name: '同时发送页面 / 选区图片' })).toHaveCount(0)
}

async function sentImageContext(page: Page, index: number) {
  const request = await page.evaluate(index => window.__studyTest.requests[index], index)
  expect(request.imageDataUrl).toMatch(/^data:image\/(png|jpeg);base64,/)
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', request.imageDataUrl!)
  const context = JSON.parse(request.userPrompt.slice(request.userPrompt.indexOf('\n') + 1))
  for (const field of ['text', 'pageText', 'recognizedText', 'extractedText', 'correctedText', 'pageTextTruncated']) expect(context).not.toHaveProperty(field)
  expect(context.context).toMatch(/image/i)
  expect(request.systemPrompt).not.toContain('RESPONSE FORMAT')
  expect(request.model).toBe('test-vision')
  return { request, context }
}

// Material, rendering, cropping and storage are real. Teaching replies remain
// visibly labelled fixed examples; they do not establish live-model math quality.
test('student journey sends the original MIT calculus crop directly without extracted or corrected source text', async ({ page, request }, info) => {
  test.setTimeout(90_000)
  const download = await request.get(SOURCE_PDF, { timeout: 45_000 })
  expect(download.ok(), `Official PDF unavailable: ${download.status()}`).toBeTruthy()
  const bytes = await download.body()
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(SOURCE_SHA256)
  await page.goto('/tests/study-browser/index.html?lang=zh')
  await page.evaluate(() => { window.__studyTest.lesson = 'mit-5b-13' })
  await page.getByLabel('导入 PDF 或图片').setInputFiles({ name: 'MIT18_01SC_pset5prb.pdf', mimeType: 'application/pdf', buffer: bytes })
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveAttribute('max', '7')
  await jumpToPage(page, 2)
  // Coordinates come from visually inspecting the public PDF, not extracting math.
  await selectSourceCrop(page, { x: 0.18, y: 0.80, width: 0.31, height: 0.05 })
  await page.screenshot({ path: info.outputPath('real-calculus-source-selection.png'), fullPage: true })
  await page.locator('.kv-study-reader-preview img').screenshot({ path: info.outputPath('real-calculus-original-crop.png') })

  // No hand-transcribed problem or correction is supplied as material context.
  await page.getByLabel('关于本页的问题').fill('5B-13：请看框选的题目，下一步该怎么做？')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByText('演示提示（非真实模型调用）', { exact: false })).toBeVisible()
  const first = await sentImageContext(page, 0)
  expect(first.request.systemPrompt).toContain('HINT MODE')
  expect(first.context.pageNumber).toBe(2)
  expect(first.context.attempt).toBeNull()
  expect(first.request.userPrompt).not.toContain('x^2/(1+x^6)')
  expect(first.request.userPrompt).not.toContain('u=x^3')
  await page.screenshot({ path: info.outputPath('real-calculus-hint.png'), fullPage: true })

  await page.getByRole('radio', { name: '检查我的解答', exact: true }).click()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeDisabled()
  await page.getByLabel('我的解答或思路').fill(ATTEMPT)
  await page.getByLabel('关于本页的问题').fill('5B-13：请检查我的换元，为什么答案不一样？')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByText('第一处问题在第三行', { exact: false })).toBeVisible()
  const second = await sentImageContext(page, 1)
  expect(second.request.systemPrompt).toContain('CHECK MODE')
  expect(second.context.attempt).toBe(ATTEMPT)
  expect(second.request.imageDataUrl).toBe(first.request.imageDataUrl)
  await page.locator('.kv-study-notes > summary').click()
  await page.getByLabel('本页笔记').fill('换元时必须一起替换微分：x² dx = du/3。下次先验算导数。')
  await page.screenshot({ path: info.outputPath('real-calculus-check-attempt.png'), fullPage: true })

  await jumpToPage(page, 3)
  await expect(page.getByLabel('我的解答或思路')).toHaveValue('')
  await expect(page.locator('.kv-study-reader-preview')).not.toHaveAttribute('open', '')
  await page.locator('.kv-study-history-row').filter({ hasText: '5B-13：请检查我的换元' }).click()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('2')
  await expect(page.getByLabel('我的解答或思路')).toHaveValue(ATTEMPT)
  await expect(page.getByText('保存在此设备', { exact: true })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('2')
  await expect(page.locator('.kv-study-reader-region')).toBeVisible()
  await expect(page.getByLabel('我的解答或思路')).toHaveValue(ATTEMPT)
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', first.request.imageDataUrl!)
  await expect(page.getByLabel('修正后的题目文字')).toHaveCount(0)
  await page.locator('.kv-study-notes > summary').click()
  await expect(page.getByLabel('本页笔记')).toHaveValue('换元时必须一起替换微分：x² dx = du/3。下次先验算导数。')

  await page.evaluate(() => { window.__studyTest.lesson = 'mit-5b-13'; window.__studyTest.failNext = true })
  await page.getByRole('radio', { name: '检查我的解答', exact: true }).click()
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByText('Simulated provider unavailable. Try again.')).toBeVisible()
  await expect(page.getByLabel('我的解答或思路')).toHaveValue(ATTEMPT)
  await page.getByRole('button', { name: '重试此问题' }).click()
  await page.getByRole('button', { name: '重新发送', exact: true }).click()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeEnabled()
  const retry = await sentImageContext(page, 1)
  expect(retry.request.imageDataUrl).toBe(first.request.imageDataUrl)
  expect(retry.context.attempt).toBe(ATTEMPT)
  await page.locator('.kv-study-reader-preview > summary').scrollIntoViewIfNeeded()
  await page.screenshot({ path: info.outputPath('real-calculus-restored-and-retried.png'), fullPage: true })
  await info.attach('real-calculus-observations.json', { body: JSON.stringify({
    source: SOURCE_PAGE, pdf: SOURCE_PDF, sha256: SOURCE_SHA256, sourceBytes: bytes.length, pageCount: 7,
    exercise: '5B-13', page: 2, officialSolutionSource: SOLUTIONS, officialSolutionPage: 6,
    sourceContext: 'Exact original page crop image; no OCR, extracted text, or corrected source transcription',
    hintContextPage: first.context.pageNumber, selectedImageMatchesPreview: true, checkAttemptPreserved: true,
    pageRegionNotesAndDraftRestored: true, providerFailureAndRetryPreservedAttemptAndImage: true,
    teachingValidation: 'Fixed simulated replies, not live-model tutoring or grading validation',
  }, null, 2), contentType: 'application/json' })
})

test('a distinct original integration-by-parts crop uses direct image and attempt with simulated feedback', async ({ page, request }, info) => {
  const download = await request.get(SOURCE_PDF)
  expect(download.ok()).toBeTruthy()
  const bytes = await download.body()
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(SOURCE_SHA256)
  await page.goto('/tests/study-browser/index.html?lang=zh')
  await page.evaluate(() => { window.__studyTest.lesson = 'mit-5f-2a' })
  await page.getByLabel('导入 PDF 或图片').setInputFiles({ name: 'MIT18_01SC_pset5prb.pdf', mimeType: 'application/pdf', buffer: bytes })
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveAttribute('max', '7')
  await jumpToPage(page, 5)
  await selectSourceCrop(page, { x: 0.20, y: 0.86, width: 0.15, height: 0.031 })
  await page.locator('.kv-study-reader-preview img').screenshot({ path: info.outputPath('real-calculus-second-original-crop.png') })
  await page.getByRole('radio', { name: '检查我的解答', exact: true }).click()
  await page.getByLabel('关于本页的问题').fill('5F-2(a)：请检查分部积分的步骤。')
  const attempt = 'u=x, dv=e^x dx\ndu=dx, v=e^x\nI=x e^x+∫e^x dx\nI=(x+1)e^x+C'
  await page.getByLabel('我的解答或思路').fill(attempt)
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByText('演示检查（非真实模型调用）', { exact: false })).toBeVisible()
  await expect(page.getByText('第三行的加号应为减号。', { exact: false })).toBeVisible()
  const call = await sentImageContext(page, 0)
  expect(call.context.pageNumber).toBe(5)
  expect(call.context.attempt).toBe(attempt)
  await expect(page.getByText('自己修正第三行', { exact: false })).toBeVisible()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeEnabled()
  await expect(page.getByText('保存在此设备', { exact: true })).toBeVisible()
  await page.screenshot({ path: info.outputPath('real-calculus-second-exercise.png'), fullPage: true })
  await page.reload()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('5')
  await expect(page.getByText('第三行的加号应为减号。', { exact: false })).toBeVisible()
  await expect(page.getByLabel('我的解答或思路')).toHaveValue(attempt)
  await expect(page.getByText('自己修正第三行', { exact: false })).toBeVisible()
  await expect(page.locator('.kv-study-reader-preview img')).toHaveAttribute('src', call.request.imageDataUrl!)
})
