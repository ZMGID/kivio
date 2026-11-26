import { expect, test } from '@playwright/test'

function expectTextCaret(editor, offset) {
  return expect.poll(() => editor.evaluate(element => {
    const selection = getSelection()
    if (!selection?.isCollapsed || !element.contains(selection.focusNode)) return null
    const before = document.createRange()
    before.selectNodeContents(element)
    before.setEnd(selection.focusNode, selection.focusOffset)
    return before.toString().length
  })).toBe(offset)
}

test('shows the local agent icon or the Kivio mascot beside the empty chat greeting', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('kivio.chat.lastAgentRuntime', JSON.stringify({ kind: 'external', externalAgentId: 'codex' }))
  })
  await page.goto('/#chat')
  const icon = page.locator('.chat-empty-hero-heading img')
  const mascot = page.getByRole('button', { name: '墨墨 · Momo' })
  await expect(icon).toHaveAttribute('src', '/agent-icons/codex.svg')
  await expect(mascot).toHaveCount(0)
  await page.locator('.kv-agent-picker > button').click()
  await page.getByRole('radio', { name: 'Kivio Chat', exact: true }).click()
  await expect(mascot).toBeVisible()
  await expect(icon).toHaveCount(0)
  await page.getByRole('radio', { name: 'Kivio Agent', exact: true }).click()
  await expect(mascot).toBeVisible()
  await expect(icon).toHaveCount(0)
})

test('keeps the empty chat centered when the window gets shorter', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await page.addInitScript(() => {
    localStorage.setItem('kivio.chat.lastAgentRuntime', JSON.stringify({ kind: 'builtin' }))
    localStorage.setItem('kivio-chat-dock-open', '1')
  })
  await page.setViewportSize({ width: 1055, height: 600 })
  await page.goto('/#chat')
  const hero = page.locator('.chat-empty-hero')
  const stack = hero.locator('.chat-empty-hero-stack')
  const editor = hero.getByRole('textbox')
  await expect(editor).toBeVisible()
  await expect(page.getByRole('button', { name: '面板', exact: true })).toHaveAttribute('aria-expanded', 'true')
  const centerOffset = () => stack.evaluate(element => {
    const { top, height } = element.getBoundingClientRect()
    const pane = element.parentElement.getBoundingClientRect()
    return top + height / 2 - (pane.top + pane.height / 2)
  })
  const normalOffset = await centerOffset()

  for (const height of [550, 480, 400, 600]) {
    await page.setViewportSize({ width: 1055, height })
    await expect.poll(async () => Math.abs(await centerOffset() - normalOffset)).toBeLessThanOrEqual(2)
    await expect(editor).toBeInViewport({ ratio: 1 })
  }
})

test('sends the renumbered image reference with the remaining files', async ({ page }) => {
  await page.addInitScript(() => {
    const paths = ['C:/screenshots/first.png', 'C:/screenshots/second.png', 'C:/screenshots/notes.pdf']
    const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII='
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'chat' }, currentWebview: { label: 'chat' } },
      transformCallback: () => 1,
      invoke(command, args) {
        if (command === 'plugin:dialog|open') return Promise.resolve(paths)
        if (command === 'chat_read_clipboard_files') return Promise.resolve({
          success: true, files: [{ path: paths[2] }],
        })
        if (command === 'chat_inspect_attachment_paths') return Promise.resolve(args.paths.map(path => ({
          path, name: path.split('/').at(-1), type: path.endsWith('.png') ? 'image' : 'file', sizeBytes: 2048,
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
  const composerFontSize = await composer.evaluate(element => getComputedStyle(element).fontSize)
  await expect(composer.getByTitle('first.png', { exact: true })).toHaveText('first.png 2.0 KB')
  await expect(composer.getByText('first.png', { exact: true })).toHaveCSS('font-size', composerFontSize)
  await expect(composer.getByTitle('second.png', { exact: true })).toHaveText('second.png 2.0 KB')
  await expect(composer.getByText('notes.pdf')).toBeVisible()
  await expect(page.getByText('#2', { exact: true })).toBeVisible()

  await page.getByRole('button', { name: '移除', exact: true }).first().click()
  await expect(composer.getByTitle('second.png', { exact: true })).toBeVisible()
  await expect(composer.getByTitle('first.png', { exact: true })).toHaveCount(0)

  await composer.focus()
  await page.keyboard.press('End')
  await page.keyboard.insertText(' 看这张')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  const sent = await page.evaluate(() => window.chatComposerSent)
  expect(sent.content).toBe('123[图片 #1] 看这张')
  expect(sent.attachments.map(attachment => attachment.name)).toEqual(['second.png', 'notes.pdf'])
  await expect(composer.getByTitle('second.png', { exact: true })).toHaveCount(0)

  await composer.click()
  await composer.fill('甲乙')
  await expectTextCaret(composer, 2)
  await page.keyboard.press('ArrowLeft')
  await expectTextCaret(composer, 1)
  await composer.evaluate(editor => {
    const clipboardData = new DataTransfer()
    clipboardData.setData('text/plain', 'C:/screenshots/notes.pdf')
    editor.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }))
  })
  await expect(composer).toHaveText('甲notes.pdf乙')
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

test('pastes over the captured selection and keeps later typing after a pending image', async ({ page }) => {
  await page.addInitScript(() => {
    const pending = new Map()
    window.hasPendingImagePaste = name => pending.has(name)
    window.finishImagePaste = name => pending.get(name)()
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'chat' }, currentWebview: { label: 'chat' } },
      transformCallback: () => 1,
      invoke(command, args) {
        if (command === 'chat_read_clipboard_files') return Promise.resolve({ success: true, files: [] })
        if (command === 'chat_save_pasted_image') {
          return new Promise(resolve => {
            pending.set(args.name, () => {
              pending.delete(args.name)
              resolve({ success: true, name: args.name, path: `C:/screenshots/${args.name}` })
            })
          })
        }
        return Promise.resolve(1)
      },
    }
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  })
  await page.goto('/scripts/fixtures/chat-composer.html')
  const composer = page.getByRole('textbox')
  const paste = async name => {
    await composer.evaluate((editor, name) => {
      const clipboardData = new DataTransfer()
      clipboardData.items.add(new File(['image'], name, { type: 'image/png' }))
      editor.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }))
    }, name)
    await expect.poll(() => page.evaluate(name => window.hasPendingImagePaste(name), name)).toBe(true)
  }
  const finish = name => page.evaluate(name => window.finishImagePaste(name), name)

  await composer.fill('前删后')
  await page.keyboard.press('Home')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('Shift+ArrowRight')
  await paste('selection.png')
  await finish('selection.png')
  await expect(composer).toHaveText('前selection.png 5 B后')

  await page.keyboard.press('End')
  await paste('pending-first.png')
  await paste('pending-second.png')
  await page.keyboard.insertText('续写')
  await finish('pending-second.png')
  await expect(composer.getByTitle('pending-second.png', { exact: true })).toBeVisible()
  await finish('pending-first.png')
  await expect(composer).toHaveText('前selection.png 5 B后pending-first.png 5 Bpending-second.png 5 B续写')

  await page.keyboard.press('End')
  await page.keyboard.press('Shift+ArrowLeft')
  await paste('edited-selection.png')
  await page.keyboard.insertText('写的内容')
  await finish('edited-selection.png')
  await expect(composer).toHaveText('前selection.png 5 B后pending-first.png 5 Bpending-second.png 5 B续edited-selection.png 5 B写的内容')
})

test('pastes a selectable attachment at the caret and supports editing on both sides', async ({ page }) => {
  await page.addInitScript(() => {
    const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII='
    const thumbnailReads = new Map()
    window.thumbnailReadCount = name => thumbnailReads.get(`C:/screenshots/${name}`) ?? 0
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'chat' }, currentWebview: { label: 'chat' } },
      transformCallback: () => 1,
      invoke(command, args) {
        if (command === 'chat_read_clipboard_files') return Promise.resolve({ success: true, files: [] })
        if (command === 'chat_read_attachment' && args.thumbnail) {
          const reads = thumbnailReads.get(args.path) ?? 0
          thumbnailReads.set(args.path, reads + 1)
          const result = { success: true, data: image }
          if (reads === 0 && args.path.endsWith('/pasted.png')) {
            return new Promise(resolve => { window.finishThumbnail = () => resolve(result) })
          }
          return Promise.resolve(result)
        }
        if (command === 'chat_save_pasted_image') {
          const saved = { success: true, path: 'C:/screenshots/' + args.name, name: args.name }
          if (args.name !== 'pasted.png') return Promise.resolve(saved)
          return new Promise(resolve => { window.finishImagePaste = () => resolve(saved) })
        }
        return Promise.resolve(1)
      },
    }
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  })
  await page.goto('/scripts/fixtures/chat-composer.html')
  const composer = page.getByRole('textbox')
  await composer.click()
  await composer.fill('前后')
  await expectTextCaret(composer, 2)
  await page.keyboard.press('ArrowLeft')
  await expectTextCaret(composer, 1)
  await composer.evaluate(editor => {
    const clipboardData = new DataTransfer()
    clipboardData.items.add(new File(['image'], 'pasted.png', { type: 'image/png' }))
    editor.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }))
  })
  await expect.poll(() => page.evaluate(() => typeof window.finishImagePaste)).toBe('function')
  await page.keyboard.press('Home')
  await expectTextCaret(composer, 0)
  await page.keyboard.insertText('新')
  await page.keyboard.press('End')
  await expectTextCaret(composer, 3)
  await page.keyboard.insertText('续')
  await page.evaluate(() => window.finishImagePaste())
  const chip = composer.getByTitle('pasted.png', { exact: true })
  await expect(composer).toHaveText('新前pasted.png 5 B后续')
  const inlinePreview = chip.locator('.chat-composer-attachment-preview')
  const inlineThumbnail = inlinePreview.locator('img')
  const fontSize = await composer.evaluate(element => getComputedStyle(element).fontSize)
  await expect(inlinePreview).toHaveCSS('width', fontSize)
  await expect(inlineThumbnail).toBeHidden()
  const chipBeforePreview = await chip.boundingBox()
  await expect.poll(() => page.evaluate(() => typeof window.finishThumbnail)).toBe('function')
  await page.evaluate(() => window.finishThumbnail())
  const preview = page.getByRole('button', { name: '预览图片' }).locator('img')
  await expect(preview).toBeVisible()
  const thumbnailSource = await preview.getAttribute('src')
  await expect(inlineThumbnail).toBeVisible()
  await expect(inlineThumbnail).toHaveJSProperty('naturalWidth', 1)
  expect(await chip.boundingBox()).toEqual(chipBeforePreview)

  await page.keyboard.press('Home')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  await expectTextCaret(composer, 2)
  await page.keyboard.insertText('左')
  await expect(composer).toHaveText('新前左pasted.png 5 B后续')
  await expect(preview).toHaveAttribute('src', thumbnailSource)
  await expect(inlineThumbnail).toHaveAttribute('src', thumbnailSource)
  await expect(inlineThumbnail).toHaveCSS('width', fontSize)
  expect(await page.evaluate(() => window.thumbnailReadCount('pasted.png'))).toBe(1)
  await chip.click()
  await expect(chip).toHaveClass(/ProseMirror-selectednode/)
  await page.keyboard.press('ArrowRight')
  await page.keyboard.insertText('右')
  await expect(composer).toHaveText('新前左pasted.png 5 B右后续')

  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('Shift+ArrowLeft')
  await expect(chip).toHaveClass(/is-selected/)
  await page.keyboard.press('ArrowRight')
  await chip.click()
  await expect(chip).toHaveClass(/ProseMirror-selectednode/)
  await page.keyboard.press('Delete')
  await expect(chip).toHaveCount(0)
  await expect(page.getByRole('button', { name: '预览图片' })).toHaveCount(0)
  await page.keyboard.press('ControlOrMeta+z')
  await expect(composer).toHaveText('新前左pasted.png 5 B右后续')

  await page.keyboard.press('End')
  await page.keyboard.insertText(' 对比图1和图2')
  await page.keyboard.press('Home')
  await expectTextCaret(composer, 0)
  await composer.evaluate(editor => {
    const clipboardData = new DataTransfer()
    clipboardData.items.add(new File(['image'], 'inserted-first.png', { type: 'image/png' }))
    editor.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }))
  })
  await expect(composer.getByTitle('inserted-first.png', { exact: true })).toHaveText('inserted-first.png 5 B')
  await expect(composer.getByTitle('pasted.png', { exact: true })).toHaveText('pasted.png 5 B')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  const sent = await page.evaluate(() => window.chatComposerSent)
  expect(sent.content).toBe('[图片 #1]新前左[图片 #2]右后续 对比图1和图2')
  expect(sent.attachments.map(attachment => attachment.name)).toEqual(['inserted-first.png', 'pasted.png'])
})
