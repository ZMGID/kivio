/// <reference types="vitest" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vitejs.dev/config/
export default defineConfig({
  base: './',
  plugins: [react()],
  build: {
    target: 'esnext',
  },
  test: {
    // 路由底色回归需要真实 CSS；其余样式仍按默认跳过。
    css: { include: [/chat-01-main\.css/] },
    environment: 'node',
    environmentMatchGlobs: [
      ['src/**/*.test.tsx', 'jsdom'],
    ],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    setupFiles: ['./src/test/setup.ts'],
  },
})
