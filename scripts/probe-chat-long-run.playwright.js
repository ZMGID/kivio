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
      // Detach bottom follow with actual user input before inspecting history.
      await page.hover('.chat-scroll-viewport');
      await page.mouse.wheel(0, -4000);
      const earlier = page.getByRole('button', { name: /显示更早的过程/ });
      await earlier.waitFor({ state: 'visible' });
      const recentText = await page.locator('.chat-markdown').allTextContents();
      await earlier.click();
      const revealedText = await page.locator('.chat-markdown').allTextContents();
      if (!recentText.every(text => revealedText.includes(text))) throw new Error('Paging discarded visible work');
      if (!revealedText.some(text => !recentText.includes(text) && text.startsWith('Step '))) {
        throw new Error('Earlier work could not be revealed');
      }
    }
  }
  await page.evaluate(reports => { window.chatLongRunReport = reports; }, reports);
}
