import { expect, test } from '@playwright/test'

test('shows attachment labels in the draft and sends the original text with the remaining files', async ({ page }) => {
  await page.addInitScript(() => {
    const paths = ['C:/screenshots/first.png', 'C:/screenshots/second.png', 'C:/screenshots/notes.pdf']
    const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII='
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'chat' }, currentWebview: { label: 'chat' } },
      transformCallback: () => 1,
      invoke(command) {
        if (command === 'plugin:dialog|open') return Promise.resolve(paths)
        if (command === 'chat_inspect_attachment_paths') return Promise.resolve(paths.map(path => ({
          path, name: path.split('/').at(-1), type: path.endsWith('.png') ? 'image' : 'file',
        })))
        if (command === 'chat_read_attachment') return Promise.resolve({ success: true, data: image })
        return Promise.resolve(1)
      },
    }
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  })
  await page.goto('/scripts/fixtures/chat-composer.html')
  const composer = page.getByRole('textbox')
  await composer.fill('123')
  await page.getByRole('button', { name: '添加', exact: true }).click()
  await page.getByRole('button', { name: '添加附件', exact: true }).click()
  await expect(page.getByRole('button', { name: '预览图片' })).toHaveCount(2)
  await expect(composer.getByText('图片 #1')).toHaveAttribute('title', 'first.png')
  await expect(composer.getByText('图片 #2')).toHaveAttribute('title', 'second.png')
  await expect(composer.getByText('notes.pdf')).toBeVisible()
  await expect(page.getByText('#2', { exact: true })).toBeVisible()

  await page.getByRole('button', { name: '移除', exact: true }).first().click()
  await expect(composer.getByText('图片 #1')).toHaveAttribute('title', 'second.png')
  await expect(composer.getByText('图片 #2')).toHaveCount(0)

  await composer.focus()
  await page.keyboard.press('End')
  await page.keyboard.insertText(' 看这张')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  const sent = await page.evaluate(() => window.chatComposerSent)
  expect(sent.content).toBe('123 看这张')
  expect(sent.attachments.map(attachment => attachment.name)).toEqual(['second.png', 'notes.pdf'])
  await expect(composer.getByText('图片 #1')).toHaveCount(0)
})

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
