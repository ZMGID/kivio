import { expect, test } from '@playwright/test'

const fixture = '/scripts/fixtures/chat-performance.html'
const viewport = '.chat-scroll-viewport'

async function show(page, id) {
  await page.evaluate(id => window.chatAcceptance.show(id), id)
}

// Read the visible row after scroll, ResizeObserver and content hydration settle.
// The expected row and offset are deliberately absent from the readiness check.
async function readingPosition(page) {
  return page.locator(viewport).evaluate(async view => {
    let previous = ''
    let stableFrames = 0
    for (;;) {
      await new Promise(requestAnimationFrame)
      const top = view.getBoundingClientRect().top
      const row = [...view.querySelectorAll('[data-chat-reading-row]')].find(row => {
        const bounds = row.getBoundingClientRect()
        return bounds.bottom > top && bounds.top < top + view.clientHeight
      })
      if (!row || view.classList.contains('is-navigator-locking')
        || view.querySelector('[data-chat-heavy-hydrated="false"], [data-chat-async-pending="true"]')
        || document.fonts.status === 'loading') {
        stableFrames = 0
        continue
      }
      const position = {
        messageId: row.dataset.messageId, y: row.getBoundingClientRect().top - top,
        top: view.scrollTop, height: view.scrollHeight,
        gap: view.scrollHeight - view.clientHeight - view.scrollTop,
      }
      const current = JSON.stringify(position)
      stableFrames = current === previous ? stableFrames + 1 : 0
      if (stableFrames >= 3) return position
      previous = current
    }
  })
}

async function readHistory(page, id) {
  await page.goto(fixture)
  await page.waitForFunction(() => window.chatAcceptance)
  await show(page, id)
  const bottom = await readingPosition(page)
  expect(bottom.gap).toBeLessThanOrEqual(2)
  // Wheel in the message column margin so nested code blocks cannot consume it.
  await page.mouse.move(12, 350)
  await page.mouse.wheel(0, -Math.floor(bottom.top / 2))
  await page.getByRole('button', { name: '回到底部' }).waitFor()
  const before = await readingPosition(page)
  expect(before.top).toBeGreaterThan(0)
  expect(before.gap).toBeGreaterThan(900)
  expect(before.messageId).toBeTruthy()
  return before
}

for (const id of ['F1', 'F2']) {
  for (const resize of [false, true]) {
    test(`${id} returns to the same visible row and offset${resize ? ' after a width change' : ''}`, async ({ page }) => {
      const before = await readHistory(page, id)
      const other = id === 'F1' ? 'F2' : 'F1'
      await show(page, other)
      // Switch back before the resize observer can deliver the new column width.
      await page.evaluate(({ id, resize }) => {
        if (resize) document.querySelector('.chat-main-pane').style.width = '760px'
        window.chatAcceptance.show(id)
      }, { id, resize })
      const after = await readingPosition(page)
      expect(after.messageId).toBe(before.messageId)
      expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(2)
    })
  }
}

test('a new scroll takes over an active restoration and returning to the bottom resumes following', async ({ page }) => {
  await readHistory(page, 'F1')
  await show(page, 'F2')
  const userPosition = await page.evaluate(async () => {
    window.chatAcceptance.show('F1')
    // Send the new reading gesture on the first frame of the restored conversation.
    await new Promise(requestAnimationFrame)
    const view = document.querySelector('.chat-scroll-viewport')
    view.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }))
    view.scrollTop -= 120
    const top = view.getBoundingClientRect().top
    const row = [...view.querySelectorAll('[data-chat-reading-row]')].find(row => row.getBoundingClientRect().bottom > top)
    return { messageId: row.dataset.messageId, y: row.getBoundingClientRect().top - top }
  })
  const settled = await readingPosition(page)
  expect(settled.messageId).toBe(userPosition.messageId)
  expect(Math.abs(settled.y - userPosition.y)).toBeLessThanOrEqual(2)
  await page.getByRole('button', { name: '回到底部' }).click()
  expect((await readingPosition(page)).gap).toBeLessThanOrEqual(2)
  await show(page, 'F2')
  await show(page, 'F1')
  expect((await readingPosition(page)).gap).toBeLessThanOrEqual(2)
})

test('turn navigation fits beside the text at the fractional container breakpoint', async ({ page }) => {
  await page.goto(fixture)
  const pane = page.locator('.chat-main-pane')
  const navigator = page.getByRole('complementary', { name: '对话轮次导航' })
  await expect(navigator).toBeVisible()
  for (const width of [719.5, 720, 999.5, 1000]) {
    await pane.evaluate((pane, width) => { pane.style.width = `${width}px` }, width)
    const list = page.locator('.chat-message-list')
    expect((await list.boundingBox()).width).toBe(width)
    if (width < 720) {
      await expect(navigator).toHaveCount(0)
      await expect(page.locator('.chat-message-navigator')).toBeHidden()
    } else {
      await expect(navigator).toBeVisible()
      const rail = await navigator.boundingBox()
      const text = await page.locator('[data-chat-rows-root]').boundingBox()
      expect(rail.x).toBeGreaterThanOrEqual(text.x + text.width)
    }
  }
})

test('sidebar and dock backgrounds fade closed at their open widths, and settings covers the sidebar immediately', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' })
  await page.goto('/scripts/fixtures/chat-titlebar.html')
  for (const [side, label] of [['sidebar', '收起侧栏'], ['dock', '面板']]) {
    const layer = page.locator(`.chat-titlebar-panel-bg--${side}`)
    const toggle = page.getByRole('button', { name: label })
    await expect(layer).toHaveCSS('opacity', '1')
    const initial = await layer.boundingBox()
    if (side === 'dock') expect(initial.width).toBe(420)
    else expect(initial.width).toBeGreaterThan(0)
    // Sample a real CSS transition in the same page task as the click. Advancing
    // its timeline avoids machine speed deciding which frame the assertion sees.
    const during = await toggle.evaluate(async (button, side) => {
      button.click()
      await Promise.resolve()
      const layer = document.querySelector(`.chat-titlebar-panel-bg--${side}`)
      const opacity = layer.getAnimations().find(animation => animation.transitionProperty === 'opacity')
      if (opacity) {
        opacity.pause()
        opacity.currentTime = opacity.effect.getTiming().duration / 2
      }
      const result = { opacity: Number(getComputedStyle(layer).opacity), width: layer.getBoundingClientRect().width }
      opacity?.finish()
      return result
    }, side)
    expect(during.opacity).toBeGreaterThan(0)
    expect(during.opacity).toBeLessThan(1)
    expect(during.width).toBe(initial.width)
    await expect(layer).toHaveCSS('opacity', '0')
  }

  const sidebar = page.locator('.chat-titlebar-panel-bg--sidebar')
  await page.getByRole('button', { name: '展开侧栏' }).click()
  await expect(sidebar).toHaveCSS('opacity', '1')
  const covered = await page.getByRole('button', { name: '设置' }).evaluate(async button => {
    button.click()
    await Promise.resolve()
    return getComputedStyle(document.querySelector('.chat-titlebar-panel-bg--sidebar')).opacity
  })
  expect(covered).toBe('0')
})

test('opens an offscreen heading in a saved CLI reply without mounting the whole reply', async ({ page }) => {
  await page.goto(fixture)
  await page.waitForFunction(() => window.chatAcceptance)
  await show(page, 'savedWork')
  const heading = page.getByRole('heading', { name: 'Inspection 46' })
  const link = page.getByRole('button', { name: '跳转到：Inspection 46' })
  await expect(link).toBeAttached()
  await expect(heading).toHaveCount(0)
  await link.click()
  await expect(heading).toBeInViewport()
  await expect(page.getByText('Inspected local file 46.', { exact: true })).toBeInViewport()
  await expect(page.getByText('Inspected local file 300.', { exact: true })).toHaveCount(0)
})
