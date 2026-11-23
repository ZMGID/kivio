import { expect, test } from '@playwright/test'

// The trailing status line (墨墨) during and after generation: it follows the answer
// smoothly while there is room, stays fixed in its slot once the answer reaches the
// bottom, never covers the newest text, and does not jump when the turn settles.
// Frame-timing assertions: a starved CPU can drop a frame and look like a jump, while a
// real regression fails on every attempt.
test.describe.configure({ retries: 2 })

const fixture = '/scripts/fixtures/chat-status-line.html'
const MASCOT_PX = 28
// One painted frame may move at most this far while following (a new paragraph lands
// ~54px at once; the gap above the mascot absorbs most of it).
const MAX_FRAME_MOVE_PX = 18
// Settling may ease across a longer distance (gap removed, row handed to history); an
// ease-out's first frame moves a bit further, but an unsmoothed snap (≥40px) must not happen.
const MAX_SETTLE_FRAME_MOVE_PX = 24

async function open(page) {
  await page.goto(fixture)
  await page.waitForFunction(() => window.statusLine && document.querySelector('.kv-stream-logo'))
  await page.evaluate(() => document.fonts.ready)
}

// Frames (after `from`) where the mascot moved further than smooth easing allows. The
// limit is per 60Hz frame and scales with elapsed time, so a dropped frame is not a jump.
function jumps(samples, from = 0, limit = MAX_FRAME_MOVE_PX) {
  const found = []
  for (let index = 1; index < samples.length; index++) {
    const [a, b] = [samples[index - 1], samples[index]]
    if (b.t < from || a.height !== b.height) continue
    const move = Math.abs(b.y - a.y)
    const allowed = limit * Math.max(1, (b.t - a.t) / (1000 / 60) - 0.5)
    if (move > allowed) found.push({ t: Math.round(b.t), dt: Math.round(b.t - a.t), from: a.y, to: b.y, scroll: [a.scrollTop, b.scrollTop], transform: b.transform })
  }
  return found
}

async function streamToBottomAndPause(page) {
  await open(page)
  await page.evaluate(() => window.statusLine.startSampling())
  const streaming = page.evaluate(() => window.statusLine.stream())
  await page.waitForFunction(() => {
    const samples = window.statusLine.samples
    const sent = samples.find(s => s.t > 450)
    return sent && samples.at(-1).sticky && samples.at(-1).scrollTop > sent.scrollTop + 50
  })
  await page.evaluate(() => window.statusLine.pause())
  await streaming
  await page.waitForTimeout(600)
}

const overlaps = samples => samples.filter(s => s.textBottom != null && s.y < s.height && s.textBottom > s.y + 1)

test('follows the answer smoothly, then stays fixed at the bottom without covering text', async ({ page }) => {
  await open(page)
  await page.evaluate(() => window.statusLine.startSampling())
  await page.evaluate(() => window.statusLine.stream())
  const samples = await page.evaluate(() => window.statusLine.stopSampling())
  // Skip the send itself (new question + reserve snap into place).
  expect(jumps(samples, 450)).toEqual([])
  expect(overlaps(samples)).toEqual([])
  // Once the answer reaches the bottom the list starts scrolling; from then on the mascot holds its slot.
  const sent = samples.find(s => s.t > 450).scrollTop
  const scrolling = samples.findIndex(s => s.t > 450 && s.scrollTop > sent + 50)
  expect(scrolling).toBeGreaterThan(0)
  const pinned = samples.slice(scrolling)
  expect(pinned.length).toBeGreaterThan(30)
  expect(pinned.every(s => s.sticky)).toBe(true)
  const slot = pinned.at(-1).y
  for (const sample of pinned) expect(Math.abs(sample.y - slot)).toBeLessThanOrEqual(1)
  expect(slot + MASCOT_PX).toBeLessThanOrEqual(pinned.at(-1).height)
})

for (const stopped of [false, true]) {
  test(`does not jump when a ${stopped ? 'stopped reply is persisted late' : 'short reply settles after a pause'}`, async ({ page }) => {
    await open(page)
    await page.evaluate(() => window.statusLine.startSampling())
    // Short enough to stay in the open space below the question, where removing the
    // breathing gap at handoff would visibly move the mascot if it were not eased.
    const streaming = page.evaluate(() => window.statusLine.stream({ paragraphs: 1 }))
    if (stopped) {
      await page.waitForTimeout(700)
      await page.evaluate(() => window.statusLine.cancel())
    }
    await streaming
    // Longer than any settle window measured from the last movement.
    await page.waitForTimeout(2200)
    const settledAt = await page.evaluate(() => window.statusLine.persist())
    await page.waitForTimeout(800)
    const samples = await page.evaluate(() => window.statusLine.stopSampling())
    expect(jumps(samples, settledAt - 50, MAX_SETTLE_FRAME_MOVE_PX)).toEqual([])
    const settled = samples.filter(s => s.t >= settledAt)
    expect(settled[0].scrollTop).toBe(settled.at(-1).scrollTop)
    // The handoff really moves the mascot here (the gap above it is removed), so the check is not vacuous.
    if (!stopped) expect(Math.abs(settled.at(-1).y - settled[0].y)).toBeGreaterThan(MAX_SETTLE_FRAME_MOVE_PX)
    const last = samples.at(-1)
    expect(last.sticky).toBe(false)
    expect(last.transform).toBe('')
  })
}

test('lets go of the bottom slot while the pointer is held and when the reader scrolls up', async ({ page }) => {
  await open(page)
  await page.evaluate(() => window.statusLine.startSampling())
  const streaming = page.evaluate(() => window.statusLine.stream())
  // Wait until the answer has reached the bottom and the list is scrolling with it.
  await page.waitForFunction(() => {
    const samples = window.statusLine.samples
    const sent = samples.find(s => s.t > 450)
    return sent && samples.at(-1).sticky && samples.at(-1).scrollTop > sent.scrollTop + 50
  })
  await page.mouse.move(640, 300)
  await page.mouse.down()
  await page.waitForTimeout(800)
  const held = await page.evaluate(() => window.statusLine.samples.slice(-30))
  expect(overlaps(held)).toEqual([])
  await page.mouse.up()
  await page.mouse.wheel(0, -600)
  await page.waitForTimeout(400)
  const detached = await page.evaluate(() => window.statusLine.samples.at(-1))
  expect(detached.sticky).toBe(false)
  expect(detached.transform).toBe('')
  await streaming
})

test('a pointer pressed during a pause releases the slot before new text arrives', async ({ page }) => {
  await streamToBottomAndPause(page)
  await page.mouse.move(640, 300)
  await page.mouse.down()
  await page.waitForTimeout(100)
  expect(await page.evaluate(() => window.statusLine.samples.at(-1).sticky)).toBe(false)
  const from = await page.evaluate(() => performance.now() - window.statusLine.t0)
  await page.evaluate(async () => {
    for (let i = 0; i < 6; i++) {
      window.statusLine.append(`\n\nHeld paragraph ${i}: more text keeps arriving while the reader holds the pointer down.`)
      await new Promise(resolve => setTimeout(resolve, 120))
    }
    window.statusLine.status('Upstream is slow, retrying the request with a longer timeout so that this status note wraps onto a second line in the reading column, then a third.')
    await new Promise(resolve => setTimeout(resolve, 600))
  })
  const held = await page.evaluate(start => window.statusLine.samples.filter(s => s.t >= start), from)
  expect(overlaps(held)).toEqual([])
  await page.mouse.up()
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
  const after = await page.evaluate(start => window.statusLine.samples.filter(s => s.t >= start), from)
  for (const sample of after) expect(Math.abs(sample.y - before.y)).toBeLessThanOrEqual(1)
})

// Trackpad-like swipe: small deltas with a momentum tail.
async function swipe(page, direction) {
  for (const delta of [4, 8, 14, 20, 24, 22, 18, 14, 10, 7, 5, 3, 2, 1]) {
    await page.mouse.wheel(0, direction * delta)
    await page.waitForTimeout(16)
  }
}

for (const [where, paragraphs] of [['in the open space', 3], ['at the bottom', 14]]) {
  test(`scrolling ${where} moves the mascot with the text instead of chasing it`, async ({ page }) => {
    await open(page)
    await page.evaluate(() => window.statusLine.startSampling())
    const streaming = page.evaluate(p => window.statusLine.stream({ paragraphs: p }), paragraphs)
    if (paragraphs > 3) {
      await page.waitForFunction(() => {
        const samples = window.statusLine.samples
        const sent = samples.find(s => s.t > 450)
        return sent && samples.at(-1).sticky && samples.at(-1).scrollTop > sent.scrollTop + 50
      })
    } else {
      await page.waitForTimeout(1500)
    }
    // Output pauses (turn still live): from here on only scrolling moves anything.
    await page.evaluate(() => window.statusLine.pause())
    await streaming
    await page.waitForTimeout(600)
    const from = await page.evaluate(() => performance.now() - window.statusLine.t0)
    await page.mouse.move(640, 400)
    await swipe(page, -1)
    await page.waitForTimeout(250)
    await swipe(page, 1)
    await swipe(page, 1)
    await page.waitForTimeout(500)
    const samples = (await page.evaluate(() => window.statusLine.stopSampling())).filter(s => s.t >= from)
    expect(samples.some(s => s.scrollTop !== samples[0].scrollTop)).toBe(true)
    const drift = []
    for (let index = 1; index < samples.length; index++) {
      const [a, b] = [samples[index - 1], samples[index]]
      if (b.y > b.height || a.height !== b.height) continue
      // Position within the content (y + scrollTop) changes only if the mascot lags the scroll.
      const moved = (b.y + b.scrollTop) - (a.y + a.scrollTop)
      // At the bottom slot the mascot is held by sticky while the list settles; that is not a chase.
      if (Math.abs(moved) > 0.5 && !(a.sticky && b.sticky && !b.transform)) drift.push({ t: Math.round(b.t), moved, transform: b.transform })
    }
    expect(drift).toEqual([])
    expect(samples.at(-1).transform).toBe('')
  })
}
