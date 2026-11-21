import { createServer } from 'vite'

// Keep dependency discovery and HMR from reloading an active browser test.
const server = await createServer({
  cacheDir: 'node_modules/.vite-browser',
  optimizeDeps: { entries: ['scripts/fixtures/chat-performance.html', 'scripts/fixtures/chat-timeline.html'] },
  server: { host: '127.0.0.1', port: 5742, strictPort: true, hmr: false, watch: null },
})
await server.listen()
