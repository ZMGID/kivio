import { expect, test } from '@playwright/test'

const fixture = '/scripts/fixtures/chat-performance.html'

async function show(page, id) {
  await page.evaluate(id => window.chatAcceptance.show(id), id)
}

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
