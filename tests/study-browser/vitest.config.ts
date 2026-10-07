import { defineConfig, mergeConfig } from 'vitest/config'
import browserConfig from './vite.config'

// A cheap real-Chat bootstrap/transport check before the browser-only PDF suite.
export default mergeConfig(browserConfig, defineConfig({
  test: {
    environment: 'jsdom', include: ['tests/study-browser/harness-smoke.test.tsx'],
    setupFiles: ['./src/test/setup.ts'], testTimeout: 30_000,
    poolOptions: { threads: { singleThread: true } },
  },
}))
