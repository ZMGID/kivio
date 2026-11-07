import assert from 'node:assert/strict'
import { once } from 'node:events'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from 'vite'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')

test('Vite watches source but excludes all Cargo build directories', async () => {
  const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, join(root, 'vite.config.ts'))
  assert.ok(loaded)
  const fixture = await mkdtemp(join(tmpdir(), 'kivio-vite-watch-'))
  let server
  let watcher
  try {
    for (const relative of ['src', 'src-tauri/target/debug', 'crates/kivio-agent-sdk/target/debug', 'tests/fixtures/agent-sdk-consumer/target/debug']) {
      const directory = join(fixture, relative)
      await mkdir(directory, { recursive: true })
      await writeFile(join(directory, 'fixture.txt'), relative)
    }
    server = await createServer({
      root: fixture,
      configFile: false,
      server: { watch: loaded.config.server.watch, middlewareMode: true, hmr: false },
      optimizeDeps: { noDiscovery: true },
    })
    await server.watcher.close()
    watcher = new server.watcher.constructor(server.watcher.options)
    const ready = once(watcher, 'ready', { signal: AbortSignal.timeout(10_000) })
    watcher.add(fixture)
    await ready
    const changed = once(watcher, 'change', { signal: AbortSignal.timeout(10_000) })
    await writeFile(join(fixture, 'src/fixture.txt'), 'updated source')
    const [changedPath] = await changed
    assert.equal(resolve(changedPath), join(fixture, 'src/fixture.txt'))
    const watched = watcher.getWatched()
    assert.deepEqual(Object.keys(watched).filter(path => path.split(/[\\/]/).includes('target')), [])
  } finally {
    await watcher?.close()
    await server?.close()
    await rm(fixture, { recursive: true, force: true })
  }
})
