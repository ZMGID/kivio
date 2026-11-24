import { expect, test } from '@playwright/test'

test('arrow keys move through wrapped text before browsing prompt history', async ({ page }) => {
  await page.goto('/scripts/fixtures/chat-composer.html')
  const composer = page.getByRole('textbox')
  const draft = '我想编辑第一行的文字，但这段草稿很长，输入框会自动把它折到下一行。'.repeat(2)
  const clickCaret = async (offset, edge) => {
    const position = await composer.evaluate((editor, { offset, edge }) => {
      const range = document.createRange()
      range.setStart(editor.firstChild, offset)
      range.setEnd(editor.firstChild, offset + 1)
      const rect = range.getBoundingClientRect()
      const bounds = editor.getBoundingClientRect()
      return { x: (edge === 'left' ? rect.left + 1 : rect.right - 1) - bounds.left, y: (rect.top + rect.bottom) / 2 - bounds.top }
    }, { offset, edge })
    await composer.click({ position })
  }
  const caretTop = () => composer.evaluate(editor =>
    getSelection().getRangeAt(0).getBoundingClientRect().top - editor.getBoundingClientRect().top + editor.scrollTop)

  await composer.fill(draft)
  const lastLine = await caretTop()
  await page.keyboard.press('ArrowUp')
  await expect(composer).toHaveText(draft)
  expect(await caretTop()).toBeLessThan(lastLine)

  await clickCaret(0, 'left')
  await page.keyboard.press('ArrowUp')
  await expect(composer).toHaveText(/^这是上一条已经发出的提问/)

  await clickCaret(1, 'left')
  const firstLine = await caretTop()
  await page.keyboard.press('ArrowDown')
  await expect(composer).toHaveText(/^这是上一条已经发出的提问/)
  expect(await caretTop()).toBeGreaterThan(firstLine)

  await clickCaret((await composer.textContent()).length - 1, 'right')
  await page.keyboard.press('ArrowDown')
  await expect(composer).toHaveText(draft)
})
