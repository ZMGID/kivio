// Run with playwright-cli run-code --filename=scripts/probe-chat-scroll.playwright.js
// Requires npm run dev:ui (or the desktop dev server) on localhost:5713.
async page => {
  const reports = [];
  for (const heavy of [false, true]) {
    await page.goto(`http://localhost:5713/scripts/fixtures/chat-scroll.html${heavy ? '?heavy' : ''}`);
    await page.waitForTimeout(1600);
    await page.evaluate(() => {
      window.chatScrollSamples = [];
      window.chatScrollSampling = true;
      let previous;
      const tick = time => {
        if (!window.chatScrollSampling) return;
        const viewport = document.querySelector('.chat-scroll-viewport');
        const bounds = viewport.getBoundingClientRect();
        const rows = [...viewport.querySelectorAll('[data-chat-item-key]')];
        const anchor = rows.find(row => {
          const rect = row.getBoundingClientRect();
          return rect.top <= bounds.top + 60 && rect.bottom > bounds.top + 60;
        });
        window.chatScrollSamples.push({
          dt: previous ? time - previous : 0,
          key: anchor?.dataset.chatItemKey,
          y: anchor?.getBoundingClientRect().top,
          rows: rows.length,
          nodes: viewport.querySelectorAll('*').length,
        });
        previous = time;
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await page.mouse.move(550, 350);
    for (let step = 0; step < 30; step++) {
      await page.mouse.wheel(0, heavy ? -9000 : -750);
      await page.waitForTimeout(40);
    }
    await page.waitForTimeout(500);
    const report = await page.evaluate(() => {
      window.chatScrollSampling = false;
      const samples = window.chatScrollSamples;
      const frames = samples.map(sample => sample.dt).sort((a, b) => a - b);
      return {
        reverseFrames: samples.slice(1).filter((sample, index) => sample.key
          && sample.key === samples[index].key && sample.y < samples[index].y - 5).length,
        blankFrames: samples.filter(sample => !sample.key).length,
        p95FrameMs: frames[Math.floor(frames.length * 0.95)],
        maxFrameMs: Math.max(...frames),
        maxRows: Math.max(...samples.map(sample => sample.rows)),
        maxNodes: Math.max(...samples.map(sample => sample.nodes)),
      };
    });
    reports.push({heavy, ...report});
  }
  await page.evaluate(reports => { window.chatScrollReport = reports; }, reports);
  if (reports.some(report => report.reverseFrames || report.blankFrames)) {
    throw new Error(`Chat scroll regression: ${JSON.stringify(reports)}`);
  }
}
