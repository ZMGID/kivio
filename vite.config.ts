/// <reference types="vitest" />
import eventSchema from './src/generated/chatProtocol.schema.json'
import syncSchema from './src/generated/chatSync.schema.json'
import Ajv2020 from 'ajv/dist/2020'
import standaloneCode from 'ajv/dist/standalone'
import { _ } from 'ajv/dist/compile/codegen'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

/** Compile generated schemas in Node; the WebView CSP forbids runtime code generation. */
function chatProtocolValidators(): Plugin {
  const moduleId = 'virtual:chat-protocol-validators'
  const resolvedId = `\0${moduleId}`
  return {
    name: 'chat-protocol-validators',
    resolveId(id) { if (id === moduleId) return resolvedId },
    load(id) {
      if (id !== resolvedId) return
      const ajv = new Ajv2020({
        allErrors: true,
        strict: false,
        code: {
          source: true,
          esm: true,
          formats: _`({double: {type: 'number', validate: Number.isFinite}})`,
        },
        formats: {
          float: true, uint8: true, uint32: true, uint64: true, int64: true,
          double: { type: 'number', validate: Number.isFinite },
        },
      })
      ajv.addSchema(eventSchema, 'event')
      ajv.addSchema(syncSchema, 'sync')
      return standaloneCode(ajv, { validateEvent: 'event', validateSync: 'sync' })
    },
  }
}

// https://vitejs.dev/config/
export default defineConfig({
  base: './',
  plugins: [chatProtocolValidators(), react()],
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
