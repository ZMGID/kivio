import { createServer } from 'vite'

// Keep dependency discovery and HMR from reloading an active browser test.
const server = await createServer({
  cacheDir: 'node_modules/.vite-browser',
  // The app entry is scanned too: fixtures reach lazily imported app code (editor, terminal,
  // Tauri APIs); without it Vite discovers those deps mid-run, re-optimizes and reloads every
  // open page, failing whichever tests are running on a cold cache.
  optimizeDeps: { entries: ['index.html', 'scripts/fixtures/chat-performance.html', 'scripts/fixtures/chat-titlebar.html', 'scripts/fixtures/chat-timeline.html', 'scripts/fixtures/chat-status-line.html'] },
  server: { host: '127.0.0.1', port: 5742, strictPort: true, hmr: false, watch: null },
})
await server.listen()
