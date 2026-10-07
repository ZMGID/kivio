import { defineConfig } from '@playwright/test'
import { fileURLToPath } from 'node:url'
const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url))
export default defineConfig({
  testDir: '.', testMatch: ['study.spec.ts', 'real-calculus.spec.ts'], timeout: 45_000, expect: { timeout: 12_000 }, fullyParallel: false, workers: 1,
  outputDir: `${repositoryRoot}test-results`,
  reporter: [['list'], ['html', { open: 'never', outputFolder: `${repositoryRoot}playwright-study-report` }]],
  use: { baseURL: 'http://127.0.0.1:5714', viewport: { width: 1500, height: 1000 }, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
  webServer: { command: 'npx vite --config tests/study-browser/vite.config.ts', cwd: repositoryRoot, url: 'http://127.0.0.1:5714/tests/study-browser/index.html', reuseExistingServer: !process.env.CI, timeout: 120_000 },
})
