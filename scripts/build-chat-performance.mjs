// node scripts/build-chat-performance.mjs [baseline-ref]
// Build the same real-component fixture against current or pre-fix renderers.
// Baseline loading is read-only; it never changes the checkout.
import { execFileSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'

const root = fileURLToPath(new URL('../', import.meta.url))
const ref = process.argv[2]
const rendererFiles = ['src/chat/MessageBubble.tsx', 'src/chat/ChatMarkdown.tsx', 'src/chat/citations.ts']
const baseline = new Map(ref ? rendererFiles.map(file => [
  path.resolve(root, file).replaceAll('\\', '/'),
  execFileSync('git', ['show', `${ref}:${file}`], { cwd: root, encoding: 'utf8' }),
]) : [])
await build({
  root,
  plugins: [{ name: 'chat-perf-baseline', enforce: 'pre', load(id) { return baseline.get(id) ?? null } }],
  build: {
    outDir: `node_modules/.cache/chat-performance/${ref ? 'baseline' : 'current'}`,
    rollupOptions: { input: path.join(root, 'scripts/fixtures/chat-performance.html') },
  },
})
