import { expect, test } from '@playwright/test'

const fixture = '/scripts/fixtures/chat-timeline.html'
const viewport = '.chat-scroll-viewport'

async function open(page) {
  await page.goto(fixture)
  await page.waitForFunction(() => window.chatTimeline)
}

async function readingPosition(page) {
  return page.locator(viewport).evaluate(view => {
    const bounds = view.getBoundingClientRect()
    const paragraph = [...view.querySelectorAll('[data-message-id=active-assistant] p')].find(element => {
      const rect = element.getBoundingClientRect()
      return rect.bottom > bounds.top && rect.top < bounds.bottom
    })
    return {
      paragraph: paragraph?.textContent,
      top: paragraph ? paragraph.getBoundingClientRect().top - bounds.top : null,
      gap: view.scrollHeight - view.clientHeight - view.scrollTop,
    }
  })
}

test('opens a search hit in its conversation and shows the matched model tab', async ({ page }) => {
  await open(page)
  await page.evaluate(() => chatTimeline.crossSearch())
  await page.waitForFunction(() => chatTimeline.events.some(event => event[0] === 'waiting'))
  await expect(page.locator('[data-message-id=alpha-answer-13]')).toBeInViewport()
  await page.evaluate(() => chatTimeline.resolveSearch())
  await expect(page.locator('[data-message-id=beta-answer-0]')).toBeInViewport()
  await expect(page.locator('[data-message-id=beta-answer-0]')).toHaveClass(/kv-search-focus-flash/)
  await page.evaluate(() => chatTimeline.sameSearch('group-answer-b'))
  await expect(page.getByText('Second answer with the search term')).toBeInViewport()
  await expect(page.locator('[data-message-id=group-answer-b]')).toHaveClass(/kv-search-focus-flash/)
  await page.getByRole('button', { name: 'Model One' }).click()
  await expect(page.getByText('First answer without the target')).toBeInViewport()
})

test('reveals a reasoning hit and falls back to the end when a search hit was deleted', async ({ page }) => {
  await open(page)
  await page.evaluate(() => chatTimeline.sameSearch('alpha-answer-0', true))
  await expect(page.getByText('Only the reasoning contains this phrase.')).toBeInViewport()
  await page.evaluate(() => chatTimeline.crossSearch('deleted-message-id'))
  await page.waitForFunction(() => chatTimeline.events.some(event => event[0] === 'waiting'))
  await page.evaluate(() => chatTimeline.resolveSearch())
  await expect(page.locator('[data-message-id=beta-answer-13]')).toBeInViewport()
  await expect.poll(async () => (await readingPosition(page)).gap).toBeLessThanOrEqual(2)
  await page.evaluate(() => chatTimeline.switchTo('alpha'))
  await expect(page.locator('[data-message-id=alpha-answer-0]').getByRole('button', { name: /^Worked/ }))
    .toHaveAttribute('aria-expanded', 'false')
})

for (const change of ['resize while away', 'settle here', 'settle while away']) {
  test(`keeps the visible live paragraph when readers ${change}`, async ({ page }) => {
    await open(page)
    await page.evaluate(process => chatTimeline.beginLong(process), change !== 'resize while away')
    await expect.poll(async () => (await readingPosition(page)).gap).toBeLessThanOrEqual(2)
    await expect(page.locator('[data-message-id=active-assistant]')).toContainText('Live paragraph 115')
    await page.mouse.move(18, 480)
    await page.mouse.wheel(0, -1300)
    await expect(page.getByRole('button', { name: '回到底部' })).toBeVisible()
    await expect.poll(async () => (await readingPosition(page)).gap).toBeGreaterThan(1000)
    const before = await readingPosition(page)
    expect(before.paragraph).toMatch(/^Live paragraph /)
    if (change !== 'settle here') {
      await page.evaluate(() => chatTimeline.switchTo('beta'))
      await expect(page.locator('[data-message-id=beta-answer-13]')).toBeInViewport()
    }
    if (change === 'resize while away') {
      await page.evaluate(() => chatTimeline.resize())
      await expect(page.locator('.chat-main-pane')).toHaveCSS('width', '740px')
    } else {
      await page.evaluate(() => { chatTimeline.finish(); chatTimeline.persist() })
    }
    if (change !== 'settle here') await page.evaluate(() => chatTimeline.switchTo('alpha'))
    if (change !== 'resize while away') {
      await expect(page.locator('[data-message-id=active-assistant]')).toHaveAttribute('data-chat-message-list-item', 'message')
    }
    await expect.poll(async () => (await readingPosition(page)).paragraph).toBe(before.paragraph)
    const after = await readingPosition(page)
    expect(after.gap).toBeGreaterThan(1000)
    expect(Math.abs(after.top - before.top)).toBeLessThanOrEqual(2)
  })
}

for (const transition of ['completion', 'stop and resume']) {
  test(`preserves the visible Work paragraph across ${transition}`, async ({ page }) => {
    await open(page)
    await page.evaluate(() => chatTimeline.beginLong(true))
    await expect(page.locator('[data-message-id=active-assistant]')).toContainText('Live paragraph 115')
    await page.mouse.move(18, 480)
    await page.mouse.wheel(0, -1300)
    await expect(page.getByRole('button', { name: '回到底部' })).toBeVisible()
    const view = page.locator(viewport)
    await view.evaluate(element => {
      const target = [...element.querySelectorAll('p')].find(paragraph =>
        paragraph.textContent.startsWith('Long step paragraph 12:'))
      element.scrollTop += target.getBoundingClientRect().top - element.getBoundingClientRect().top + 11
    })
    await expect.poll(async () => (await readingPosition(page)).paragraph).toMatch(/^Long step paragraph 12:/)
    const before = await readingPosition(page)
    async function expectRestored() {
      await expect(view).not.toHaveClass(/is-navigator-locking/)
      const after = await readingPosition(page)
      expect(after.paragraph).toBe(before.paragraph)
      expect(Math.abs(after.top - before.top)).toBeLessThanOrEqual(2)
    }
    if (transition === 'completion') {
      await page.evaluate(() => { chatTimeline.finish(); chatTimeline.persist() })
      await expect(page.locator('[data-message-id=active-assistant]')).toHaveAttribute('data-chat-message-list-item', 'message')
      await expectRestored()
    } else {
      await page.evaluate(() => chatTimeline.freeze())
      await expectRestored()
      await page.evaluate(() => chatTimeline.switchTo('beta'))
      await expect(page.locator('[data-message-id=beta-answer-13]')).toBeInViewport()
      await page.evaluate(() => { chatTimeline.resume(); chatTimeline.switchTo('alpha') })
      await expectRestored()
    }
  })
}

test('keeps the current question and previous answer after switching chats', async ({ page }) => {
  await open(page)
  await page.evaluate(() => chatTimeline.prompt())
  await page.getByRole('option', { name: /^(\d+ )?First answer$/ }).click()
  await expect(page.getByRole('listbox', { name: 'Second question' })).toBeVisible()
  await page.evaluate(() => chatTimeline.switchTo('beta'))
  await expect(page.getByRole('listbox')).toHaveCount(0)
  await page.evaluate(() => chatTimeline.switchTo('alpha'))
  await expect(page.getByRole('listbox', { name: 'Second question' })).toBeVisible()
  await page.getByRole('button', { name: '上一题' }).click()
  await expect(page.getByRole('option', { name: /^(\d+ )?First answer$/ })).toHaveAttribute('aria-selected', 'true')
  await page.evaluate(() => { chatTimeline.resolvePrompt(); chatTimeline.prompt('second-prompt') })
  await expect(page.getByRole('listbox', { name: 'First question' })).toBeVisible()
  await expect(page.getByRole('option', { name: /^(\d+ )?First answer$/ })).toHaveAttribute('aria-selected', 'false')
})

test('keeps an answer in flight across chat switches and permits retry after failure', async ({ page }) => {
  await open(page)
  await page.evaluate(() => chatTimeline.promptWithDeferredAnswer())
  await page.getByRole('option', { name: /Option one/ }).click()
  await page.waitForFunction(() => pendingAnswers.length === 1)
  await page.evaluate(() => chatTimeline.switchTo('beta'))
  await expect(page.getByRole('listbox')).toHaveCount(0)
  await page.evaluate(() => chatTimeline.switchTo('alpha'))
  const option = page.getByRole('option', { name: /Option two/ })
  await expect(option).toBeDisabled()
  await page.evaluate(() => pendingAnswers[0].reject(new Error('Answer could not be sent')))
  await expect(page.getByText('Answer could not be sent')).toBeVisible()
  await option.click()
  await page.waitForFunction(() => pendingAnswers.length === 2)
  await page.evaluate(() => pendingAnswers[1].resolve())
  await expect(page.getByRole('listbox')).toHaveCount(0)
})

for (const history of [
  { label: 'Single', conversation: 'alpha', other: 'beta', message: 'alpha-answer-13' },
  { label: 'Grouped', conversation: 'beta', other: 'alpha', message: 'group-answer-a' },
]) {
  test(`keeps manually opened Work and tool details in ${history.label.toLowerCase()} history`, async ({ page }) => {
    await page.goto(`${fixture}?history-work`)
    await page.waitForFunction(() => window.chatTimeline)
    const row = page.locator(`[data-message-id=${history.message}]`)
    if (history.conversation === 'beta') {
      await page.evaluate(() => { chatTimeline.switchTo('beta'); return chatTimeline.sameSearch('group-answer-a') })
      await expect(row).toHaveClass(/kv-search-focus-flash/)
      await expect(page.locator(viewport)).not.toHaveClass(/is-navigator-locking/)
    }
    const worked = row.getByRole('button', { name: /^Worked/ })
    await expect(worked).toHaveAttribute('aria-expanded', 'false')
    await worked.click()
    await row.locator('[data-tool-call-cluster] button[data-chat-disclosure]').first().click()
    await row.locator('[data-work-scroll] button[data-chat-disclosure]').first().click()
    const output = row.getByText(`${history.label} tool result 1`, { exact: true })
    await expect(output).toBeVisible()
    async function switchAwayAndBack() {
      await page.evaluate(id => chatTimeline.switchTo(id), history.other)
      await expect(row).toHaveCount(0)
      await page.evaluate(id => chatTimeline.switchTo(id), history.conversation)
    }
    await switchAwayAndBack()
    await expect(worked).toHaveAttribute('aria-expanded', 'true')
    await expect(output).toBeVisible()
    await worked.click()
    await switchAwayAndBack()
    await expect(worked).toHaveAttribute('aria-expanded', 'false')
  })
}
