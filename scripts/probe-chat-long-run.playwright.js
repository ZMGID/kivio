// Open the current production chat-performance.html fixture before running.
// Save window.chatLongRunReport afterwards. No models or backend writes.
async page => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.waitForFunction(() => Boolean(window.chatAcceptance));
  const reports = [];
  for (const steps of [300, 1000]) {
    for (const update of ['text', 'tool']) {
      const result = await page.evaluate(({ steps, update }) => window.chatAcceptance.longRun(steps, update), { steps, update });
      if (result.domNodes > 12000) throw new Error(`Long-run default DOM exceeded budget: ${result.domNodes}`);
      await page.getByText('Current output', { exact: false }).waitFor({ state: 'visible' });
      if (result.liveBubbleTransform !== 'none') throw new Error('Finished entrance still retains a transformed live bubble');
      await page.evaluate(() => window.chatAcceptance.resize());
      await page.waitForTimeout(300);
      const bottomGap = await page.evaluate(() => {
        const viewport = document.querySelector('.chat-scroll-viewport');
        return viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop;
      });
      if (Math.abs(bottomGap) > 3) throw new Error(`Long run lost bottom anchoring after resize: ${bottomGap}`);
      reports.push({ ...result, bottomGap });
      // Each timing sample uses the same full width; the narrow layout is only
      // a separate scroll/resize assertion, never the next sample's baseline.
      await page.evaluate(() => window.chatAcceptance.resize());
      await page.waitForTimeout(300);
      // Read a middle step by scrolling the same timeline the user sees.
      const viewport = page.locator('.chat-scroll-viewport');
      const distance = await viewport.evaluate(view => Math.floor(view.scrollHeight / 2));
      await page.mouse.move(12, 350);
      await page.mouse.wheel(0, -distance);
      await page.waitForFunction(steps => {
        const view = document.querySelector('.chat-scroll-viewport').getBoundingClientRect();
        return [...document.querySelectorAll('.chat-markdown')].some(row => {
          const match = /^Step (\d+):/.exec(row.textContent);
          const rect = row.getBoundingClientRect();
          return match && Number(match[1]) > steps / 8 && Number(match[1]) < 7 * steps / 8
            && rect.bottom > view.top && rect.top < view.bottom;
        });
      }, steps);
      if (await page.locator('*').count() > 12000) throw new Error('Scrolling mounted the entire work history');
      await page.getByRole('button', { name: '回到底部' }).click();
      await page.waitForFunction(() => {
        const view = document.querySelector('.chat-scroll-viewport');
        return view.scrollHeight - view.clientHeight - view.scrollTop <= 3;
      });
    }
  }
  await page.evaluate(reports => { window.chatLongRunReport = reports; }, reports);
}
