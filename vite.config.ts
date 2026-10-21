/// <reference types="vitest" />
import eventSchema from './src/generated/chatProtocol.schema.json'
import syncSchema from './src/generated/chatSync.schema.json'
import Ajv2020 from 'ajv/dist/2020'
import standaloneCode from 'ajv/dist/standalone'
import { _ } from 'ajv/dist/compile/codegen'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import { readFileSync, readdirSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'

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

/** Serve and bundle PDF.js resources locally; filenames must retain their CMap names. */
function pdfResources(): Plugin {
  const root = dirname(createRequire(import.meta.url).resolve('pdfjs-dist/package.json'))
  const assets = new Map<string, Buffer>()
  for (const directory of ['cmaps', 'standard_fonts', 'wasm']) {
    for (const name of readdirSync(join(root, directory))) {
      assets.set(`${directory}/${name}`, readFileSync(join(root, directory, name)))
    }
  }
  return {
    name: 'pdfjs-local-resources',
    configureServer(server) {
      server.middlewares.use('/pdfjs', (request, response, next) => {
        const key = new URL(request.url ?? '/', 'http://localhost').pathname.slice(1)
        const data = assets.get(key)
        if (!data) { next(); return }
        response.setHeader('Content-Type', key.endsWith('.wasm') ? 'application/wasm' : 'application/octet-stream')
        response.end(data)
      })
    },
    generateBundle() {
      for (const [name, source] of assets)
        this.emitFile({ type: 'asset', fileName: `pdfjs/${name}`, source })
    },
  }
}

// https://vitejs.dev/config/
export default defineConfig({
  base: './',
  plugins: [chatProtocolValidators(), pdfResources(), react()],
  build: {
    target: 'esnext',
  },
  test: {
    // 路由底色回归需要真实 CSS；其余样式仍按默认跳过。
    css: { include: [/chat-01-main\.css/] },
    environment: 'node',
    environmentMatchGlobs: [
      ['src/**/*.test.tsx', 'jsdom'],
      ['extensions/**/*.test.tsx', 'jsdom'],
    ],
    include: ['src/**/*.{test,spec}.{ts,tsx}', 'extensions/**/*.{test,spec}.{ts,tsx}'],
    setupFiles: ['./src/test/setup.ts'],
  },
})
