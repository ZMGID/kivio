import { expect, test } from '@playwright/test'

// The trailing status line (墨墨) during generation: once the answer reaches the bottom
// while following, it stays fixed in its bottom slot and the text scrolls up past it,
// never covering the newest line. Leaving the bottom or holding the pointer lets go of
// the slot, so it never floats over history. Finishing at the bottom does not move it.

const fixture = '/scripts/fixtures/chat-status-line.html'
const MASCOT_PX = 28
const LONG_REPLY = 14

async function open(page) {
  await page.goto(fixture)
  await page.waitForFunction(() => window.statusLine && document.querySelector('.kv-stream-logo'))
  await page.evaluate(() => document.fonts.ready)
}

// The answer has reached the bottom and the list is scrolling with it. The baseline is the
// first frame with answer text (the send and its reserve have settled by then), not a fixed
// time, so a slow page start cannot shift it.
const scrollingWithAnswer = () => {
  const samples = window.statusLine.samples
  const sent = samples.find(s => s.textBottom != null)
  return Boolean(sent) && samples.at(-1).sticky && samples.at(-1).scrollTop > sent.scrollTop + 50
}

async function streamToBottomAndPause(page) {
  await open(page)
  await page.evaluate(() => window.statusLine.startSampling())
  const streaming = page.evaluate(p => window.statusLine.stream({ paragraphs: p }), LONG_REPLY)
  await page.waitForFunction(scrollingWithAnswer)
  await page.evaluate(() => window.statusLine.pause())
  await streaming
  await page.waitForTimeout(600)
}

const now = page => page.evaluate(() => performance.now() - window.statusLine.t0)
const samplesSince = (page, from) => page.evaluate(start => window.statusLine.samples.filter(s => s.t >= start), from)
const overlaps = samples => samples.filter(s => s.textBottom != null && s.y < s.height && s.textBottom > s.y + 1)

test('a short answer keeps the same spacing above the mascot while streaming and after saving', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await open(page)
  await page.evaluate(() => {
    window.statusLine.begin()
    window.statusLine.append('这是一个只有一行的回答。')
  })
  const measureGap = async kind => {
    const paragraph = page.locator(`[data-chat-message-list-item="${kind}"] p`).last()
    await expect(paragraph).toHaveText('这是一个只有一行的回答。')
    return paragraph.evaluate(p => {
      const range = document.createRange()
      range.selectNodeContents(p)
      return document.querySelector('.kv-stream-logo').getBoundingClientRect().top - range.getBoundingClientRect().bottom
    })
  }
  const activeGap = await measureGap('streaming')
  await page.evaluate(() => window.statusLine.persist())
  // The persisted answer enters virtual history and is measured after it mounts.
  await expect.poll(async () => Math.abs(activeGap - await measureGap('message'))).toBeLessThanOrEqual(1)
})

for (const [pace, chunk] of [['steady', 12], ['bursty', 160]]) test(`stays fixed at the bottom while a ${pace} answer scrolls up, without covering text`, async ({ page }) => {
  await open(page)
  await page.evaluate(() => window.statusLine.startSampling())
  // Bursts land several lines at once, so the list's scroll glide falls furthest behind.
  await page.evaluate(([p, c]) => window.statusLine.stream({ paragraphs: p, chunk: c, interval: c > 12 ? 120 : 40 }), [LONG_REPLY * (chunk > 12 ? 2 : 1), chunk])
  const samples = await page.evaluate(() => window.statusLine.stopSampling())
  expect(overlaps(samples)).toEqual([])
  const first = samples.findIndex(s => s.textBottom != null)
  expect(first).toBeGreaterThanOrEqual(0)
  const scrolling = samples.findIndex((s, index) => index > first && s.scrollTop > samples[first].scrollTop + 50)
  expect(scrolling).toBeGreaterThan(0)
  const pinned = samples.slice(scrolling)
  expect(pinned.length).toBeGreaterThan(30)
  expect(pinned.some((s, i) => i > 0 && s.gap > 1 && s.gap < pinned[i - 1].gap && s.scrollTop > pinned[i - 1].scrollTop)).toBe(true)
  expect(pinned.every(s => s.sticky)).toBe(true)
  const slot = pinned.at(-1).y
  for (const sample of pinned) expect(Math.abs(sample.y - slot)).toBeLessThanOrEqual(1)
  expect(slot + MASCOT_PX).toBeLessThanOrEqual(pinned.at(-1).height)
})

for (const stopped of [false, true]) {
  test(`does not move when a ${stopped ? 'stopped' : 'finished'} reply at the bottom is persisted`, async ({ page }) => {
    await streamToBottomAndPause(page)
    if (stopped) {
      await page.evaluate(() => window.statusLine.cancel())
      await page.waitForTimeout(1200)
    }
    const before = await page.evaluate(() => window.statusLine.samples.at(-1))
    const settledAt = await page.evaluate(() => window.statusLine.persist())
    await page.waitForTimeout(800)
    const after = await samplesSince(page, settledAt)
    for (const sample of after) expect(Math.abs(sample.y - before.y)).toBeLessThanOrEqual(1)
    expect(after.at(-1).sticky).toBe(false)
  })
}

test('lets go of the bottom slot while the pointer is held and when the reader scrolls up', async ({ page }) => {
  await open(page)
  await page.evaluate(() => window.statusLine.startSampling())
  const streaming = page.evaluate(p => window.statusLine.stream({ paragraphs: p }), LONG_REPLY)
  await page.waitForFunction(scrollingWithAnswer)
  await page.mouse.move(640, 300)
  await page.mouse.down()
  await page.waitForTimeout(100)
  const from = await now(page)
  await page.waitForTimeout(700)
  const held = await samplesSince(page, from)
  expect(held.every(s => !s.sticky)).toBe(true)
  expect(overlaps(held)).toEqual([])
  await page.mouse.up()
  await page.mouse.wheel(0, -600)
  await page.waitForTimeout(400)
  expect(await page.evaluate(() => window.statusLine.samples.at(-1).sticky)).toBe(false)
  await page.evaluate(() => window.statusLine.pause())
  await streaming
})

test('a pointer pressed during a pause releases the slot before new text arrives', async ({ page }) => {
  await streamToBottomAndPause(page)
  await page.mouse.move(640, 300)
  await page.mouse.down()
  await page.waitForTimeout(100)
  expect(await page.evaluate(() => window.statusLine.samples.at(-1).sticky)).toBe(false)
  const from = await now(page)
  await page.evaluate(async () => {
    for (let i = 0; i < 6; i++) {
      window.statusLine.append(`\n\nHeld paragraph ${i}: more text keeps arriving while the reader holds the pointer down.`)
      await new Promise(resolve => setTimeout(resolve, 120))
    }
    window.statusLine.status('Upstream is slow, retrying the request with a longer timeout so that this status note wraps onto a second line in the reading column, then a third.')
    await new Promise(resolve => setTimeout(resolve, 600))
  })
  expect(overlaps(await samplesSince(page, from))).toEqual([])
  await page.mouse.up()
  await expect.poll(() => page.evaluate(() => window.statusLine.samples.at(-1).gap)).toBeLessThanOrEqual(1)
})

test('parallel answers follow independently when the reader scrolls one column', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kivio.chat.multiAnswerView', 'columns'))
  await open(page)
  await page.evaluate(() => {
    window.statusLine.beginColumns()
    window.statusLine.appendColumns('A paragraph in both answers.\n\n'.repeat(48))
  })
  const columns = page.locator('.chat-message-group-col-body')
  const first = columns.nth(0)
  const second = columns.nth(1)
  const bottomGap = element => element.scrollHeight - element.clientHeight - element.scrollTop
  await expect.poll(() => second.evaluate(el => el.scrollTop)).toBeGreaterThan(500)
  await first.hover()
  await page.mouse.wheel(0, -5000)
  await expect.poll(() => first.evaluate(bottomGap)).toBeGreaterThan(250)
  await expect.poll(() => first.evaluate(el => el.scrollTop)).toBe(0)
  await page.mouse.wheel(0, -120)
  await page.evaluate(() => window.statusLine.appendColumns('More output.\n\n'.repeat(12) + 'Last line'))
  await expect(first).toContainText('Last line')
  await expect(second).toContainText('Last line')
  await expect.poll(() => second.evaluate(bottomGap)).toBeLessThanOrEqual(1)
  expect(await first.evaluate(el => el.scrollTop)).toBe(0)
  await expect.poll(() => page.locator('.kv-stream-logo').last()
    .evaluate(el => Boolean(el.closest('[style*="sticky"]')))).toBe(true)
  await page.mouse.wheel(0, 5000)
  await expect.poll(() => first.evaluate(bottomGap)).toBeLessThanOrEqual(1)
  await page.evaluate(() => window.statusLine.appendColumns('\n\nFollowing again'))
  await expect(first).toContainText('Following again')
  await expect.poll(() => first.evaluate(bottomGap)).toBeLessThanOrEqual(1)
  await page.evaluate(() => window.statusLine.persistColumns())
  await expect(page.locator('[data-chat-message-list-item="group"]')).toBeVisible()
  await expect.poll(() => first.evaluate(bottomGap)).toBeLessThanOrEqual(1)
  await expect.poll(() => second.evaluate(bottomGap)).toBeLessThanOrEqual(1)
})

test('toggling Working at the live end keeps the arriving answer in view', async ({ page }) => {
  await open(page)
  await page.evaluate(() => {
    window.statusLine.begin()
    window.statusLine.append('Checking a file.', {id: 'step', kind: 'text', phase: 'tool_loop', order: 0})
    window.statusLine.completeTool()
  })
  await page.getByRole('button', {name: /^Working/}).click()
  await page.evaluate(() => {
    window.statusLine.append('A finding.\n\n'.repeat(40) + 'Final finding.', {
      id: 'findings', kind: 'text', phase: 'synthesis', order: 2,
    })
  })
  await expect(page.getByText('Final finding.', {exact: true})).toBeInViewport()
})

test('a wrapping status note does not move the mascot out of its bottom slot', async ({ page }) => {
  await streamToBottomAndPause(page)
  const before = await page.evaluate(() => window.statusLine.samples.at(-1))
  expect(before.sticky).toBe(true)
  const from = await page.evaluate(() => {
    window.statusLine.status('Upstream is slow, retrying the request with a longer timeout so that this status note wraps onto a second line in the reading column, then a third.')
    return performance.now() - window.statusLine.t0
  })
  await page.waitForTimeout(900)
  for (const sample of await samplesSince(page, from)) expect(Math.abs(sample.y - before.y)).toBeLessThanOrEqual(1)
})

// Trackpad-like swipe: small deltas with a momentum tail.
async function swipe(page, direction) {
  for (const delta of [4, 8, 14, 20, 24, 22, 18, 14, 10, 7, 5, 3, 2, 1]) {
    await page.mouse.wheel(0, direction * delta)
    await page.waitForTimeout(16)
  }
}

test('scrolling at the bottom moves the mascot with the text instead of chasing it', async ({ page }) => {
  await streamToBottomAndPause(page)
  const from = await now(page)
  await page.mouse.move(640, 400)
  await swipe(page, -1)
  await page.waitForTimeout(250)
  await swipe(page, 1)
  await swipe(page, 1)
  await page.waitForTimeout(500)
  const samples = await samplesSince(page, from)
  expect(samples.some(s => s.scrollTop !== samples[0].scrollTop)).toBe(true)
  const drift = []
  for (let index = 1; index < samples.length; index++) {
    const [a, b] = [samples[index - 1], samples[index]]
    if (b.y > b.height || a.height !== b.height || (a.sticky && b.sticky)) continue
    // Position within the content (y + scrollTop) changes only if the mascot lags the scroll.
    const moved = (b.y + b.scrollTop) - (a.y + a.scrollTop)
    if (Math.abs(moved) > 0.5) drift.push({ t: Math.round(b.t), moved })
  }
  expect(drift).toEqual([])
})
