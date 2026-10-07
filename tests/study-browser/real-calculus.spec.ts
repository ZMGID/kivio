import { createHash } from 'node:crypto'
import { test, expect } from '@playwright/test'
import { attempt, capture, captureCrop, expectImageSource, expectSharedChat, importMaterial, jumpToPage, messages, openStudy, originalImage, savedConversations, selectMode, selectSourceCrop, send, sendButton, sentSource } from './shared'

// MIT OCW / Arthur Mattuck, 18.01SC, original exercise sheet, fetched per run.
// Attribution/terms: https://ocw.mit.edu/pages/privacy-and-terms-of-use/
const PDF = 'https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/50d9ff5b7a30fe96bd69017ca5104d6e_MIT18_01SC_pset5prb.pdf'
const SHA256 = '5015876951651a4f4695ccbbb74e22fdf1a43b1798d4859ed3324cdc9e90d23d'
const ATTEMPT = '令 u=x^3\ndu=3x^2 dx\n原积分 = ∫du/(1+u^2)\n= arctan(u)+C = arctan(x^3)+C'
let bytes: Buffer
test.describe.configure({ timeout: 90_000 })
test.beforeAll(async ({ request }) => {
  const response = await request.get(PDF, { timeout: 45_000 }); expect(response.ok()).toBeTruthy()
  bytes = await response.body(); expect(createHash('sha256').update(bytes).digest('hex')).toBe(SHA256)
})
test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 1250 })
  await openStudy(page)
  await importMaterial(page, { name: 'MIT18_01SC_pset5prb.pdf', mimeType: 'application/pdf', buffer: bytes })
  await expect(page.getByRole('spinbutton', { name: '页码' })).toHaveAttribute('max', '7')
})

test('real calculus hint and check use shared Chat source attachment, attempt and ordinary retry', async ({ page }, info) => {
  await jumpToPage(page, 2)
  await selectSourceCrop(page, { x: 0.18, y: 0.80, width: 0.31, height: 0.05 })
  const image = await originalImage(page, SHA256)
  await captureCrop(page, info, 'shared-chat-calculus-original-crop.png')
  await selectMode(page, '提示一步')
  await send(page, '5B-13：请看框选的题目，下一步该怎么做？', 'mit-5b-13')
  const hint = await sentSource(page, 0, 2, 'hint', image)
  expect(hint.studySource?.attempt).toBe('')
  await expect(messages(page)).toContainText('演示提示（非真实模型调用）')
  await selectMode(page, '检查我的解答')
  await expect(attempt(page)).toHaveAttribute('aria-required', 'true')
  await expect(sendButton(page)).toBeDisabled()
  await attempt(page).fill(ATTEMPT)
  await send(page, '5B-13：请检查我的换元，为什么答案不一样？', 'mit-5b-13')
  const checked = await sentSource(page, 1, 2, 'check', image)
  expect(checked.studySource?.attempt).toBe(ATTEMPT)
  await expect(messages(page)).toContainText('第一处问题在第三行')
  await capture(page, info, 'shared-chat-calculus-check.png')
  await expect(page.getByText('材料草稿保存在此设备', { exact: true })).toBeVisible()
  await page.reload()
  await expectSharedChat(page)
  await expectImageSource(page.locator('.kv-study-reader-preview img'), image)
  await selectMode(page, '检查我的解答')
  await expect(attempt(page)).toHaveValue(ATTEMPT)
  await page.evaluate(() => { window.__studyTest.lesson = 'mit-5b-13'; window.__studyTest.failNext = true })
  const input = page.locator('[data-composer-presentation="reading"] .chat-composer-editor[role="textbox"]')
  await input.fill('再检查这次换元')
  await sendButton(page).click()
  await expect(messages(page)).toContainText('Simulated provider unavailable')
  await page.getByRole('button', { name: '重试', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.__studyTest.requests.length)).toBe(2)
  const retried = await sentSource(page, 1, 2, 'check', image)
  expect(retried.kind).toBe('retry')
  expect(retried.studySource?.attempt).toBe(ATTEMPT)
  await expect.poll(async () => (await savedConversations(page)).find(item => item.id === hint.conversationId)?.messages.at(-1)?.stream_outcome).toBe('completed')
  await capture(page, info, 'shared-chat-calculus-retry.png')
})

test('a second original math crop checks an attempt and full solutions use the shared disclosure', async ({ page }, info) => {
  await jumpToPage(page, 5)
  await selectSourceCrop(page, { x: 0.20, y: 0.86, width: 0.15, height: 0.031 })
  const image = await originalImage(page, SHA256)
  await selectMode(page, '检查我的解答')
  await attempt(page).fill('u=x, dv=e^x dx\ndu=dx, v=e^x\nI=x e^x+∫e^x dx\nI=(x+1)e^x+C')
  await send(page, '5F-2(a)：请检查分部积分的步骤。', 'mit-5f-2a')
  await sentSource(page, 0, 5, 'check', image)
  await expect(messages(page)).toContainText('第三行的加号应为减号')
  await selectMode(page, '完整解答')
  await page.evaluate(() => { window.__studyTest.rawReply = '演示完整解答（非真实模型调用）：按分部积分公式保留正确的减号。' })
  await send(page, '现在请给出完整解答。')
  await expect(page.getByRole('button', { name: '展开完整解答', exact: true })).toBeVisible()
  await expect(page.getByText('演示完整解答（非真实模型调用）', { exact: false })).toBeHidden()
  await page.getByRole('button', { name: '展开完整解答', exact: true }).click()
  await expect(page.getByText('演示完整解答（非真实模型调用）', { exact: false })).toBeVisible()
  await capture(page, info, 'shared-chat-calculus-solution-disclosure.png')
})
