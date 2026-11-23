import { expect, test } from '@playwright/test'

test('opening the titlebar picker keeps the pane still and its models reachable after resize', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 400 })
  await page.goto('/scripts/fixtures/agent-model-picker.html')
  const trigger = page.getByRole('button', { name: /Kivio Agent/ })
  const dialog = page.getByRole('dialog')
  await trigger.waitFor()
  const before = await trigger.boundingBox()
  await trigger.click()
  await expect(dialog.getByRole('textbox')).toBeFocused()
  const after = await trigger.boundingBox()
  expect(after.y).toBe(before.y)
  expect(after.x).toBe(before.x)

  for (const size of [{ width: 640, height: 400 }, { width: 760, height: 450 }]) {
    await page.setViewportSize(size)
    await expect.poll(async () => {
      const menu = await dialog.boundingBox()
      const button = await trigger.boundingBox()
      return menu.x >= 0 && menu.y >= button.y + button.height
        && menu.x + menu.width <= size.width && menu.y + menu.height <= size.height
    }).toBe(true)
  }
  const search = dialog.getByRole('textbox')
  await search.fill('kimi-k3')
  await dialog.getByRole('button', { name: /kimi.k3/i }).first().click()
  await expect(dialog).toBeHidden()
  await expect(trigger).toHaveAccessibleName(/kimi.k3/i)
  await trigger.click()
  const current = dialog.locator('[aria-current=true]').first()
  await expect(current).toBeInViewport({ ratio: 1 })
  await current.click()
  await expect(dialog).toBeHidden()
})
