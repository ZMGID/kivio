// Run with playwright-cli run-code --filename=scripts/probe-chat-history.playwright.js
// Uses real MessageList geometry and delayed synthetic pages; no model calls.
async page => {
  page.setDefaultTimeout(10000);
  await page.route('**/*', route => route.continue());
  await page.goto('http://localhost:5713/scripts/fixtures/chat-scroll.html?history');
  await page.waitForTimeout(1200);
  const initial = await page.evaluate(() => ({ ...window.chatHistoryPaging }));
  if (initial.requests !== 0) throw new Error('Opening the latest page fetched older history');
  if (await page.getByRole('button', {name: '加载更早消息', exact: true}).count()) {
    throw new Error('The obsolete history button is still present');
  }
  const viewport = page.locator('.chat-scroll-viewport');
  const box = await viewport.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const anchor = async () => page.evaluate(() => {
    const viewport = document.querySelector('.chat-scroll-viewport');
    const top = viewport.getBoundingClientRect().top;
    const row = [...viewport.querySelectorAll('[data-chat-item-key]')].find(row => {
      const rect = row.getBoundingClientRect();
      return rect.top <= top + 60 && rect.bottom > top + 60;
    });
    return row ? { key: row.dataset.chatItemKey, offset: row.getBoundingClientRect().top - top } : null;
  });
  const drifts = [];
  for (let count = 1; count <= 3; count++) {
    const movement = await viewport.evaluate(e => Math.max(0, e.scrollTop - e.clientHeight * 1.5));
    await page.mouse.wheel(0, -movement);
    await page.waitForFunction(count => window.chatHistoryPaging.requests === count, count);
    // Chromium animates wheel movement; sample after the gesture settles,
    // while the delayed page is still in flight.
    await page.waitForTimeout(100);
    const before = await anchor();
    if (await page.evaluate(() => window.chatHistoryPaging.commits) !== count - 1) {
      throw new Error('Anchor was sampled after the page committed');
    }
    if (!before) throw new Error('No visible row before prepend');
    await page.waitForFunction(count => window.chatHistoryPaging.commits === count, count);
    await page.waitForTimeout(450);
    const offset = await page.evaluate(key => {
      const viewport = document.querySelector('.chat-scroll-viewport');
      const row = [...viewport.querySelectorAll('[data-chat-item-key]')].find(row => row.dataset.chatItemKey === key);
      return row ? row.getBoundingClientRect().top - viewport.getBoundingClientRect().top : null;
    }, before.key);
    const drift = offset === null ? null : offset - before.offset;
    drifts.push(drift);
    if (drift === null || Math.abs(drift) > 3) throw new Error(`Prepend moved the reading row: ${drift}px`);
    const requests = await page.evaluate(() => window.chatHistoryPaging.requests);
    if (requests !== count) throw new Error('A pending page produced duplicate reads');
  }
  await page.mouse.wheel(0, -100000);
  await page.waitForTimeout(500);
  const final = await page.evaluate(() => ({ ...window.chatHistoryPaging }));
  if (final.start !== 0 || final.requests !== 3) throw new Error(`Unexpected final paging state: ${JSON.stringify(final)}`);

  // A failed read leaves the viewport intact and the next upward gesture retries.
  await page.goto('http://localhost:5713/scripts/fixtures/chat-scroll.html?history');
  await page.waitForTimeout(1200);
  await page.evaluate(() => { window.chatHistoryPaging.failNext = true; });
  await page.mouse.wheel(0, -100000);
  await page.getByRole('alert').waitFor();
  const failed = await page.evaluate(() => ({ ...window.chatHistoryPaging }));
  if (failed.requests !== 1 || failed.start !== 30) throw new Error('Failed read changed history or retried itself');
  await page.mouse.wheel(0, -100);
  await page.waitForFunction(() => window.chatHistoryPaging.commits === 1);
  await page.waitForTimeout(450);
  const retried = await page.evaluate(() => ({ ...window.chatHistoryPaging }));
  if (retried.requests !== 2 || retried.start !== 20 || await page.getByRole('alert').count()) {
    throw new Error(`History retry did not recover: ${JSON.stringify(retried)}`);
  }
  await page.evaluate(report => { window.chatHistoryReport = report; }, {drifts, final, failed, retried});
}
