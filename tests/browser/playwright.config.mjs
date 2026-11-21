import { fileURLToPath } from 'node:url'
import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: '.',
  outputDir: '../../output/browser-tests',
  use: { baseURL: 'http://127.0.0.1:5742', viewport: { width: 1280, height: 900 } },
  webServer: {
    command: 'node tests/browser/serve.mjs',
    cwd: fileURLToPath(new URL('../../', import.meta.url)),
    url: 'http://127.0.0.1:5742/scripts/fixtures/chat-performance.html',
  },
})
