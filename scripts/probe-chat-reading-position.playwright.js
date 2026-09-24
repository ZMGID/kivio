// Requires npm run dev:ui; uses only the existing synthetic chat fixtures.
// playwright-cli -s=chat-reading run-code --filename=scripts/probe-chat-reading-position.playwright.js
async page => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('http://127.0.0.1:5713/scripts/fixtures/chat-performance.html');
  await page.waitForFunction(() => Boolean(window.chatAcceptance));
  await page.evaluate(() => document.fonts.ready);
  const reports = [];
  for (const [id, resize] of [['F1', false], ['F2', false], ['F1', true], ['F2', true]]) {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.evaluate(id => window.chatAcceptance.show(id), id);
    await page.waitForTimeout(600);
    await page.evaluate(() => {
      const viewport = document.querySelector('.chat-scroll-viewport');
      viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -500, bubbles: true }));
      viewport.scrollTop = Math.floor((viewport.scrollHeight - viewport.clientHeight) / 2);
      viewport.dispatchEvent(new Event('scroll'));
    });
    await page.waitForTimeout(500);
    const before = await page.evaluate(() => {
      const viewport = document.querySelector('.chat-scroll-viewport');
      const top = viewport.getBoundingClientRect().top;
      const row = [...viewport.querySelectorAll('[data-chat-item-key]')]
        .find(row => row.getBoundingClientRect().bottom > top);
      return { scrollTop: viewport.scrollTop, key: row.dataset.chatItemKey, y: row.getBoundingClientRect().top - top };
    });
    await page.evaluate(id => window.chatAcceptance.show(id === 'F1' ? 'F2' : 'F1'), id);
    await page.waitForTimeout(100);
    const saved = await page.evaluate(async id => {
      const { recallChatReadingPosition } = await import('/src/chat/chatReadingPosition.ts');
      return recallChatReadingPosition(id);
    }, id);
    // Below max-w-4xl so the message column actually reflows.
    if (resize) await page.setViewportSize({ width: 760, height: 900 });
    await page.evaluate(id => window.chatAcceptance.show(id), id);
    await page.waitForTimeout(800);
    const after = await page.evaluate(key => {
      const viewport = document.querySelector('.chat-scroll-viewport');
      const row = [...viewport.querySelectorAll('[data-chat-item-key]')]
        .find(row => row.dataset.chatItemKey === key);
      return { scrollTop: viewport.scrollTop, y: row ? row.getBoundingClientRect().top - viewport.getBoundingClientRect().top : null };
    }, before.key);
    reports.push({ id, resize, before, saved, after });
  }
  await page.evaluate(reports => { window.chatReadingPositionReport = reports; }, reports);
  const failures = reports.filter(({ before, saved, after }) => !saved || saved.following
    || Math.abs(saved.scrollTop - before.scrollTop) > 1
    || after.y === null || Math.abs(after.y - before.y) > 2);
  if (failures.length) throw new Error(`Reading position regression: ${JSON.stringify(failures)}`);

  // A new reading gesture must retain ownership after restoration.
  const userTop = await page.evaluate(() => {
    const viewport = document.querySelector('.chat-scroll-viewport');
    viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }));
    viewport.scrollTop -= 120;
    viewport.dispatchEvent(new Event('scroll'));
    return viewport.scrollTop;
  });
  await page.waitForTimeout(300);
  const laterTop = await page.evaluate(() => document.querySelector('.chat-scroll-viewport').scrollTop);
  if (Math.abs(laterTop - userTop) > 2) throw new Error(`Restoration overrode the reader: ${userTop} -> ${laterTop}`);

  // A reader who left at the bottom must still follow after returning.
  await page.getByRole('button', { name: '回到底部', exact: true }).click();
  await page.waitForTimeout(800);
  await page.evaluate(() => window.chatAcceptance.show('F1'));
  await page.waitForTimeout(100);
  await page.evaluate(() => window.chatAcceptance.show('F2'));
  await page.waitForTimeout(800);
  const gap = await page.evaluate(() => {
    const viewport = document.querySelector('.chat-scroll-viewport');
    return viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop;
  });
  if (gap > 2) throw new Error(`Following was lost on return: ${gap}px`);
}
