// playwright-cli -s=chat-acceptance run-code --filename=scripts/probe-chat-acceptance.playwright.js
async page => {
  await page.setViewportSize({width: 1280, height: 900});
  await page.goto('http://localhost:5713/scripts/fixtures/chat-performance.html');
  await page.waitForFunction(() => Boolean(window.chatAcceptance));
  await page.waitForTimeout(500);
  const reports = [];
  for (const id of ['F2', 'F3', 'F1', 'F4', 'tools']) {
    const interactionMs = await page.evaluate(id => {
      window.chatAcceptance.reset();
      const start = performance.now();
      window.chatAcceptance.show(id);
      return performance.now() - start;
    }, id);
    await page.waitForTimeout(1000);
    if (id === 'tools') {
      await page.getByRole('button', {name: /^Worked/}).click();
      await page.waitForTimeout(200);
      const steps = await page.getByText(/^step_\d+$/, {exact: true}).count();
      if (steps !== 20) throw new Error(`Expected 20 initial process cards, got ${steps}`);
      await page.getByRole('button', {name:'显示更早的过程（480）'}).click();
      if (await page.getByText(/^step_\d+$/, {exact:true}).count() !== 40) throw new Error('Process paging failed');
    }
    if (id === 'F4') await page.evaluate(() => window.chatAcceptance.stream());
    const before = await page.evaluate(() => {
      const viewport = document.querySelector('.chat-scroll-viewport');
      return {rows: viewport.querySelectorAll('[data-chat-item-key]').length, nodes:viewport.querySelectorAll('*').length};
    });
    const resizeMs = await page.evaluate(() => {
      const start = performance.now(); window.chatAcceptance.resize();
      return performance.now() - start;
    });
    await page.waitForTimeout(300);
    const metrics = await page.evaluate(() => {
      const viewport = document.querySelector('.chat-scroll-viewport');
      const report = window.chatAcceptance.report();
      return {rows:viewport.querySelectorAll('[data-chat-item-key]').length,
        nodes:viewport.querySelectorAll('*').length,
        bottomGap:viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop,
        maxCommitMs:Math.max(0,...report.buckets.map(bucket => bucket.maxActualMs)),
        longTasks:report.longTasks.length,
        maxLongTaskMs:Math.max(0,...report.longTasks.map(task => task.durationMs)),
        report};
    });
    reports.push({id, interactionMs, resizeMs, before, ...metrics});
    if (id !== 'tools' && metrics.bottomGap > 3) throw new Error(`${id} lost bottom anchoring: ${metrics.bottomGap}`);
    if (metrics.rows > 30) throw new Error(`${id} mounted too many rows: ${metrics.rows}`);
  }
  await page.evaluate(reports => {window.chatAcceptanceReport = reports}, reports);
  console.log(JSON.stringify(reports.map(({report,...summary}) => summary), null, 2));
}
