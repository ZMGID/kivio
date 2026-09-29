// Open the current production chat-performance.html fixture before running.
// Save window.chatLongRunReport afterwards. No models or backend writes.
async page => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.waitForFunction(() => Boolean(window.chatAcceptance));
  const reports = [];
  for (const steps of [300, 1000]) {
    for (const update of ['text', 'tool']) {
      const result = await page.evaluate(({ steps, update }) => window.chatAcceptance.longRun(steps, update), { steps, update });
      if (result.markdownBlocks !== steps + 1) throw new Error('Long-run fixture lost text steps');
      if (result.liveBubbleTransform !== 'none') throw new Error('Finished entrance still retains a transformed live bubble');
      await page.evaluate(() => window.chatAcceptance.resize());
      await page.waitForTimeout(300);
      const bottomGap = await page.evaluate(() => {
        const viewport = document.querySelector('.chat-scroll-viewport');
        return viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop;
      });
      if (Math.abs(bottomGap) > 3) throw new Error(`Long run lost bottom anchoring after resize: ${bottomGap}`);
      reports.push({ ...result, bottomGap });
    }
  }
  await page.evaluate(reports => { window.chatLongRunReport = reports; }, reports);
}
