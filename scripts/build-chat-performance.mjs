// node scripts/build-chat-performance.mjs [baseline-ref]
// Build the same real-component fixture against current or pre-fix renderers.
// Baseline loading never replaces tracked files; the temporary CSS is removed.
import { execFileSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync, unlinkSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'

const root = fileURLToPath(new URL('../', import.meta.url))
const ref = process.argv[2]
const rendererFiles = ['src/chat/MessageBubble.tsx', 'src/chat/ChatMarkdown.tsx', 'src/chat/citations.ts', 'src/styles/chat-01-main.css']
const baseline = new Map(ref ? rendererFiles.map(file => [
  path.resolve(root, file).replaceAll('\\', '/'),
  execFileSync('git', ['show', `${ref}:${file}`], { cwd: root, encoding: 'utf8' }),
]) : [])
const chatCssPath = path.resolve(root, 'src/styles/chat-01-main.css').replaceAll('\\', '/')
const expectedFill = (baseline.get(chatCssPath) ?? readFileSync(chatCssPath, 'utf8'))
  .match(/animation:\s*chat-motion-bubble-in[^;]*\b(both|backwards)\s*;/)?.[1]
if (!expectedFill) throw new Error('Cannot determine the source bubble animation fill mode')
// Vite/PostCSS reads CSS @imports directly from disk, bypassing load hooks.
// Keep the overlay beside the source so relative CSS assets still resolve.
const cssOverlay = ref ? path.join(root, `src/styles/.chat-perf-${randomUUID()}.css`) : null
if (cssOverlay) {
  writeFileSync(cssOverlay, baseline.get(chatCssPath), { flag: 'wx' })
}
try {
  await build({
    root,
    plugins: [{
      name: 'chat-perf-baseline', enforce: 'pre',
      load(id) {
        if (cssOverlay && id === path.resolve(root, 'src/styles/app.css').replaceAll('\\', '/')) {
          const css = readFileSync(id, 'utf8')
          const originalImport = '@import "./chat-01-main.css";'
          if (!css.includes(originalImport)) throw new Error('Chat stylesheet import was not found')
          return css.replace(originalImport, `@import "./${path.basename(cssOverlay)}";`)
        }
        return baseline.get(id) ?? null
      },
      generateBundle(_options, bundle) {
        const css = Object.values(bundle).filter(item => item.type === 'asset' && item.fileName.endsWith('.css'))
          .map(item => item.source).join('\n')
        const actualFill = css.match(/animation:\s*chat-motion-bubble-in[^;}]*\b(both|backwards)\s*[;}]/)?.[1]
        if (actualFill !== expectedFill) throw new Error(`Chat CSS baseline mismatch: expected ${expectedFill}, got ${actualFill}`)
      },
    }],
    build: {
      outDir: `node_modules/.cache/chat-performance/${ref ? 'baseline' : 'current'}`,
      rollupOptions: { input: path.join(root, 'scripts/fixtures/chat-performance.html') },
    },
  })
} finally {
  if (cssOverlay) unlinkSync(cssOverlay)
}
