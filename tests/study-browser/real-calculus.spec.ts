import { createHash } from 'node:crypto'
import { test, expect } from '@playwright/test'

// Public source is fetched for this acceptance run, never checked in or bundled.
// MIT OCW / Arthur Mattuck, 18.01SC, Integration Techniques, exercise 5B-13.
// Source terms: https://ocw.mit.edu/pages/privacy-and-terms-of-use/
const SOURCE_PAGE = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/resources/mit18_01sc_pset5prb/'
const SOURCE_PDF = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/50d9ff5b7a30fe96bd69017ca5104d6e_MIT18_01SC_pset5prb.pdf'
const SOURCE_SHA256 = '5015876951651a4f4695ccbbb74e22fdf1a43b1798d4859ed3324cdc9e90d23d'
const SOLUTIONS = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/06979381db650b91c0de9d6755f03154_MIT18_01SC_pset5sol.pdf'
const CORRECTED = 'MIT 18.01SC, 5B-13: 求不定积分 ∫ x^2/(1+x^6) dx。题目提示：u=x^3。'
const ATTEMPT = '令 u=x^3\ndu=3x^2 dx\n原积分 = ∫du/(1+u^2)\n= arctan(u)+C = arctan(x^3)+C'

// The material/rendering/storage interactions are real. Teaching replies are
// visibly labelled, fixed examples grounded in the separate official solution.
test('student journey through a real MIT calculus PDF with honest math fallback', async ({ page, request }, info) => {
  test.setTimeout(90_000)
  const download = await request.get(SOURCE_PDF, { timeout: 45_000 })
  expect(download.ok(), `Official PDF unavailable: ${download.status()}`).toBeTruthy()
  const bytes = await download.body()
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(SOURCE_SHA256)
  await page.goto('/tests/study-browser/index.html?lang=zh')
  await page.evaluate(() => { window.__studyTest.lesson = 'mit-5b-13' })
  await page.getByLabel('导入 PDF 或图片').setInputFiles({ name: 'MIT18_01SC_pset5prb.pdf', mimeType: 'application/pdf', buffer: bytes })
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveAttribute('max', '7')
  await page.getByRole('spinbutton', { name: '页码' }).fill('2')
  await page.getByRole('spinbutton', { name: '页码' }).press('Enter')
  await expect(page.locator('.kv-study-page-label')).toHaveText('第 2 页')
  await expect(page.locator('.kv-study-reader-paper canvas')).toBeVisible()
  await page.getByText('查看本页文字（可复制）', { exact: true }).scrollIntoViewIfNeeded()
  await page.getByRole('button', { name: '框选', exact: true }).click()
  const paper = page.locator('.kv-study-reader-paper')
  const rect = await paper.boundingBox()
  expect(rect).not.toBeNull()
  // The problem row was inspected in the actual source page, not guessed from extracted math.
  await page.mouse.move(rect!.x + rect!.width * 0.18, rect!.y + rect!.height * 0.80)
  await page.mouse.down()
  await page.mouse.move(rect!.x + rect!.width * 0.49, rect!.y + rect!.height * 0.85, { steps: 12 })
  await page.mouse.up()
  await expect(page.locator('.kv-study-reader-region')).toBeVisible()
  await page.getByText('查看框选区域文字（尽力提取）', { exact: true }).click()
  const extracted = await page.locator('.kv-study-reader-extracted').innerText()
  expect(extracted.replace(/\s/g, '')).toContain('5B-13')
  expect(extracted).not.toContain('5B-12')
  expect(extracted).not.toContain('5B-14')
  await expect(page.locator('.kv-study-reader-notice')).toContainText('公式')
  await page.screenshot({ path: info.outputPath('real-calculus-source-selection.png'), fullPage: true })

  // Plain PDF text does not preserve reliable mathematical layout. The learner
  // checks the page image and supplies unambiguous text rather than trusting OCR.
  await page.getByText('补充或修正题目文字', { exact: true }).click()
  await page.getByLabel('修正后的题目文字').fill(CORRECTED)
  await page.locator('.kv-study-correction > summary').click()
  await page.getByRole('button', { name: '学习模型', exact: true }).click()
  await page.getByRole('option', { name: 'test-vision', exact: true }).click()
  await page.getByLabel('关于本页的问题').fill('5B-13：我看到了 u=x³ 的提示，下一步该怎么做？')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByText('演示提示（非真实模型调用）', { exact: false })).toBeVisible()
  const first = await page.evaluate(() => window.__studyTest.requests[0])
  expect(first.systemPrompt).toContain('HINT MODE')
  expect(first.imageDataUrl).toMatch(/^data:image\/png;base64,/)
  const context = JSON.parse(first.userPrompt.slice(first.userPrompt.indexOf('\n') + 1))
  expect(context.pageNumber).toBe(2)
  expect(context.pageText).toBe(CORRECTED)
  await page.screenshot({ path: info.outputPath('real-calculus-hint.png'), fullPage: true })

  await page.getByRole('radio', { name: '检查我的解答', exact: true }).click()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeDisabled()
  await page.getByLabel('我的解答或思路').fill(ATTEMPT)
  await page.getByLabel('关于本页的问题').fill('5B-13：请检查我的换元，为什么答案不一样？')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByText('第一处问题在第三行', { exact: false })).toBeVisible()
  const second = await page.evaluate(() => window.__studyTest.requests[1])
  expect(second.systemPrompt).toContain('CHECK MODE')
  expect(JSON.parse(second.userPrompt.slice(second.userPrompt.indexOf('\n') + 1)).attempt).toBe(ATTEMPT)
  await page.locator('.kv-study-notes > summary').click()
  await page.getByLabel('本页笔记').fill('换元时必须一起替换微分：x² dx = du/3。下次先验算导数。')
  await page.screenshot({ path: info.outputPath('real-calculus-check-attempt.png'), fullPage: true })

  await page.getByRole('spinbutton', { name: '页码' }).fill('3')
  await page.getByRole('spinbutton', { name: '页码' }).press('Enter')
  await expect(page.getByLabel('我的解答或思路')).toHaveValue('')
  await page.locator('.kv-study-history-row').filter({ hasText: '5B-13：请检查我的换元' }).click()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('2')
  await expect(page.getByLabel('我的解答或思路')).toHaveValue(ATTEMPT)
  await expect(page.getByText('保存在此设备', { exact: true })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveValue('2')
  await expect(page.locator('.kv-study-reader-region')).toBeVisible()
  await expect(page.getByLabel('我的解答或思路')).toHaveValue(ATTEMPT)
  await expect(page.getByLabel('修正后的题目文字')).toHaveValue(CORRECTED)
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
  await page.getByText('查看框选区域文字（尽力提取）', { exact: true }).scrollIntoViewIfNeeded()
  await page.screenshot({ path: info.outputPath('real-calculus-restored-and-retried.png'), fullPage: true })
  await info.attach('real-calculus-observations.json', { body: JSON.stringify({
    source: SOURCE_PAGE, pdf: SOURCE_PDF, sha256: SOURCE_SHA256, sourceBytes: bytes.length, pageCount: 7,
    exercise: '5B-13', page: 2, extractedSelection: extracted, correctedSource: CORRECTED,
    officialSolutionSource: SOLUTIONS, officialSolutionPage: 6, checkedAnswer: 'arctan(x^3)/3 + C',
    hintContextPage: context.pageNumber, hintContextMatchesCorrection: context.pageText === CORRECTED,
    selectedImageIncluded: Boolean(first.imageDataUrl), checkAttemptPreserved: true,
    pageRegionNotesAndDraftRestored: true, providerFailureAndRetryPreservedAttempt: true,
    teachingValidation: 'Fixed simulated replies, not live-model tutoring or grading validation',
  }, null, 2), contentType: 'application/json' })
})
