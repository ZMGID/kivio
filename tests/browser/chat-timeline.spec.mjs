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
    const live = page.locator('[data-message-id=active-assistant]')
    await expect(live).toContainText('Live paragraph 115')
    // Measure the reading position after the entrance transform stops moving the paragraph.
    await live.evaluate(row => Promise.all(row.getAnimations({ subtree: true })
      .filter(animation => animation.animationName === 'chat-motion-bubble-in')
      .map(animation => animation.finished)))
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
      await expect(async () => {
        const after = await readingPosition(page)
        expect(after.paragraph).toBe(before.paragraph)
        expect(Math.abs(after.top - before.top)).toBeLessThanOrEqual(2)
      }).toPass({ timeout: 5_000 })
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

test('a resumed timeline shows the model change above its question and one interruption after the answer', async ({ page }) => {
  await open(page)
  await page.setViewportSize({ width: 390, height: 640 })
  await page.evaluate(() => {
    const from = { provider: 'cli:claude', provider_name: 'Claude', model: 'opus', model_name: 'Opus 5.5', effort: 'high' }
    const to = { ...from, model: 'sonnet', model_name: 'Sonnet 5.5', effort: 'medium' }
    chatTimeline.showConversation({
      id: 'branch', forked_from: { conversation_id: 'alpha', message_id: 'excluded', display_after_message_id: 'a', title: 'Source' }, messages: [
        { id: 'q', role: 'user', timestamp: 1, content: 'Review the change', turn_model: { previous: from, current: to } },
        { id: 'a', role: 'assistant', timestamp: 2, content: 'Partial reply', stream_outcome: 'cancelled' },
        { id: 'next', role: 'user', timestamp: 3, content: 'Continue', turn_model: {
          previous: to, current: { ...to, effort: 'xhigh' },
        } },
      ],
    })
  })
  const switches = page.getByRole('separator', { name: '模型设置变更' })
  await expect(switches).toHaveCount(2)
  await expect(switches.first()).toHaveAttribute('title', 'Opus 5.5 High → Sonnet 5.5 Medium')
  await expect(switches.last()).toHaveAttribute('title', 'Medium → XHigh')
  const interrupted = page.getByRole('separator', { name: '运行已中断' })
  await expect(interrupted).toHaveCount(1)
  const box = locator => locator.boundingBox()
  expect((await box(switches.first())).y).toBeLessThan((await box(page.getByText('Review the change'))).y)
  expect((await box(interrupted)).y).toBeGreaterThan((await box(page.getByText('Partial reply'))).y)
  expect((await box(interrupted)).y).toBeLessThan((await box(switches.last())).y)
  const source = page.getByRole('button', { name: '分叉自对话 · Source' })
  expect((await box(source)).y).toBeGreaterThan((await box(interrupted)).y)
  expect((await box(source)).y).toBeLessThan((await box(switches.last())).y)
  await source.click()
  await expect(page.locator('[data-message-id=alpha-answer-13]')).toBeInViewport()
})

test('changing providers with the same model shows both provider names once', async ({ page }) => {
  await open(page)
  await page.evaluate(() => {
    const from = { provider: 'a', provider_name: 'Team A', model: 'shared', effort: 'high' }
    chatTimeline.showConversation({ id: 'providers', messages: [
      { id: 'old', role: 'user', content: 'Older question without a saved model', timestamp: 1 },
      { id: 'admitted', role: 'user', content: 'Use the second account', timestamp: 2, turn_model: {
        previous: from, current: { ...from, provider: 'b', provider_name: 'Team B' },
      } },
    ] })
  })
  const change = page.getByRole('separator', { name: '模型设置变更' })
  await expect(change).toHaveCount(1)
  await expect(change).toHaveAttribute('title', 'Team A shared → Team B shared')
})

test('only an active compaction sweeps, including when the last message already has a boundary', async ({ page }) => {
  await open(page)
  const conversation = {
    id: 'compaction', messages: [{ id: 'a', role: 'assistant', timestamp: 1, content: '已整理好对话记录。' }],
    context_state: { compaction_boundaries: [
      { id: 'first', display_after_message_id: 'a', token_estimate_before: 12000, token_estimate_after: 3000 },
    ] },
  }
  await page.evaluate(c => chatTimeline.showConversation({ ...c, compacting: true }), conversation)
  const active = page.getByRole('status', { name: '压缩中' })
  const completed = page.getByRole('separator', { name: '上下文压缩边界' })
  await expect(active).toBeInViewport()
  await expect(completed).toContainText('12.0k → 3.0k')

  const animations = row => row.evaluate(el => el.getAnimations({ subtree: true }).map(a => ({
    running: a.playState, target: a.effect.target.getAttribute('aria-hidden'),
  })))
  await expect.poll(() => animations(active)).toEqual([{ running: 'running', target: 'true' }])
  const position = () => active.locator('.chat-compaction-highlight').evaluate(el => getComputedStyle(el).maskPosition)
  const initial = await position()
  await expect.poll(position).not.toBe(initial)
  expect(await animations(completed)).toEqual([])

  for (const media of [{ reducedMotion: 'reduce' }, { reducedMotion: 'no-preference', forcedColors: 'active' }]) {
    await page.emulateMedia(media)
    await expect(active).toBeVisible()
    await expect.poll(() => animations(active)).toEqual([])
  }

  await page.emulateMedia({ forcedColors: 'none' })
  conversation.context_state.compaction_boundaries.push({
    id: 'second', display_after_message_id: 'a', token_estimate_before: 3000, token_estimate_after: 800,
  })
  await page.evaluate(c => chatTimeline.showConversation(c), conversation)
  await expect(active).toHaveCount(0)
  await expect(completed.last()).toContainText('3.0k → 800')
  expect(await animations(completed.last())).toEqual([])
})

test('shows project conversation marks through background completion, failure and retry', async ({ page }) => {
  await page.addInitScript(() => {
    const key = 'kivio-chat-dev-conversations'
    if (localStorage.getItem(key)) return
    localStorage.setItem('kivio-chat-dev-projects', JSON.stringify([
      { id: 'status-project', name: '状态验收项目', created_at: 1, updated_at: 1 },
    ]))
    localStorage.setItem(key, JSON.stringify(['status-current', 'status-other'].map((id, index) => ({
      id, title: index === 0 ? '生成状态验收' : '另一条对话',
      provider_id: 'demo', model: 'dev-model', project_id: 'status-project',
      agent_runtime: { kind: 'native' }, revision: 1, created_at: 1, updated_at: 1,
      messages: [{ id: id + '-first', role: 'user', content: '先前的问题', timestamp: 1 }],
    }))))
  })
  await page.goto('/#chat/status-current')
  await page.getByRole('button', { name: '项目', exact: true }).click()
  const row = page.getByRole('button', { name: '生成状态验收', exact: true })
  const other = page.getByRole('button', { name: '另一条对话', exact: true })
  const mark = page.locator('[data-reorder-id="status-current"] .kv-run-mark')
  await expect(row).toBeVisible()

  const controlReplies = () => page.evaluate(async () => {
    const { chatApi } = await import('/src/chat/api.ts')
    const send = chatApi.sendMessage
    chatApi.sendMessage = (...args) => new Promise((resolve, reject) => {
      window.finishStatusReply = (fail) => {
        delete window.finishStatusReply
        if (fail) reject(new Error('模型调用失败'))
        else send(...args).then(resolve, reject)
      }
    })
  })
  const startReply = async () => {
    const editor = page.locator('[contenteditable=true]').last()
    await editor.fill('测试回复状态')
    await editor.press('Enter')
    await expect(mark).toHaveAttribute('data-running', '')
    await page.waitForFunction(() => window.finishStatusReply)
  }
  const markColor = () => mark.evaluate(el => getComputedStyle(el, '::before').backgroundColor)
  const themeColor = name => page.evaluate(name => {
    const el = document.createElement('span')
    el.style.color = `var(${name})`
    document.body.append(el)
    const color = getComputedStyle(el).color
    el.remove()
    return color
  }, name)

  await controlReplies()
  await startReply()
  await other.click()
  await expect(page).toHaveURL(/status-other$/)
  await page.evaluate(() => window.finishStatusReply(false))
  await expect(row).toHaveAccessibleDescription('回复已生成，未读')
  await expect.poll(markColor).toBe(await themeColor('--color-blue-500'))
  await page.reload()
  await expect(row).toHaveAccessibleDescription('回复已生成，未读')
  await row.click()
  await expect(page).toHaveURL(/status-current$/)
  await expect(page.getByText(/这是浏览器预览模式的本地回复/)).toBeVisible()
  await expect(mark).not.toHaveAttribute('data-mark')

  await controlReplies()
  await startReply()
  await page.evaluate(() => window.finishStatusReply(true))
  await expect(row).toHaveAccessibleDescription('回复生成失败')
  await expect.poll(markColor).toBe(await themeColor('--danger'))
  await startReply()
  await page.evaluate(() => window.finishStatusReply(false))
  await expect(mark).not.toHaveAttribute('data-running')
  await expect(mark).not.toHaveAttribute('data-mark')
})

test('switches agents in a populated conversation and keeps one handoff at its original position', async ({ page }) => {
  const id = 'conv_agent_handoff'
  await page.addInitScript(id => {
    const key = 'kivio-chat-dev-conversations'
    if (localStorage.getItem(key)) return
    localStorage.setItem(key, JSON.stringify([{
      id, title: '交接验收', revision: 1, created_at: 1, updated_at: 2,
      provider_id: 'demo', model: 'chat-model',
      agent_runtime: { kind: 'external', externalAgentId: 'codex', externalModel: 'source-model' },
      messages: [
        { id: 'q', role: 'user', content: '项目代号是蓝鹭。', timestamp: 1 },
        { id: 'a', role: 'assistant', content: '记下了，代号是蓝鹭。', timestamp: 2 },
      ],
    }]))
  }, id)
  await page.goto('/#chat/' + id)
  const source = page.getByText('记下了，代号是蓝鹭。', { exact: true })
  await expect(source).toBeVisible()
  const picker = page.locator('.kv-agent-picker > button').first()
  await picker.click()
  await page.getByRole('radio', { name: 'Kivio Agent', exact: true }).click()
  const pending = page.getByRole('separator', { name: /^待交接上下文/ })
  await expect(pending).toContainText('source-model')
  // A different selection before sending replaces the destination in the same row.
  await page.getByRole('radio', { name: 'Kivio Chat', exact: true }).click()
  await expect(pending).toHaveCount(1)
  await expect(pending).toHaveAccessibleName(/Kivio Chat/)
  await page.reload({ waitUntil: 'domcontentloaded' })
  await expect(pending).toBeVisible()
  const editor = page.locator('[contenteditable=true]').last()
  await editor.fill('项目代号是什么？')
  await editor.press('Enter')
  const handoff = page.getByRole('separator', { name: /^上下文交接/ })
  await expect(handoff).toHaveCount(1)
  await expect(pending).toHaveCount(0)
  await expect(handoff).toHaveAccessibleName(/Kivio Chat/)
  const followup = page.getByText('项目代号是什么？', { exact: true })
  await expect(followup).toBeVisible()
  expect((await handoff.boundingBox()).y).toBeGreaterThan((await source.boundingBox()).y)
  expect((await handoff.boundingBox()).y).toBeLessThan((await followup.boundingBox()).y)
})
