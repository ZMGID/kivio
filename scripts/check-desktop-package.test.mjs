import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { checkMacApp, checkResources, checkVersions } from './check-desktop-package.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
function fixture(t) {
  const directory = mkdtempSync(path.join(tmpdir(), 'kivio-package-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  mkdirSync(path.join(directory, 'src-tauri'))
  for (const file of ['package.json', 'package-lock.json', 'src-tauri/Cargo.toml',
    'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json']) {
    cpSync(path.join(root, file), path.join(directory, file))
  }
  return directory
}
function changeJson(root, file, change) {
  const target = path.join(root, file)
  const value = JSON.parse(readFileSync(target, 'utf8'))
  change(value)
  writeFileSync(target, JSON.stringify(value))
}

test('all checked-in versions and resource sources agree', () => {
  const { version } = checkVersions(root)
  checkVersions(root, `v${version}`)
  assert.ok(checkResources(root) > 0)
})

test('macOS bundles request a complete signature', t => {
  const directory = fixture(t)
  changeJson(directory, 'src-tauri/tauri.conf.json', config => {
    delete config.bundle.macOS.signingIdentity
  })
  assert.throws(() => checkVersions(directory), /macOS signing identity/)
})

test('rejects a macOS bundle without its resource seal', t => {
  assert.throws(() => checkMacApp(path.join(fixture(t), 'Unsigned.app'), 'com.zmair.kivio'), /CodeResources/)
})

test('validates a real macOS signature and rejects tampering or the wrong identity', {
  skip: process.platform !== 'darwin',
}, t => {
  const directory = fixture(t)
  const app = path.join(directory, 'Test.app')
  const contents = path.join(app, 'Contents')
  mkdirSync(path.join(contents, 'MacOS'), { recursive: true })
  mkdirSync(path.join(contents, 'Resources'))
  const identifier = 'com.zmair.kivio.signature-test'
  writeFileSync(path.join(contents, 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>${identifier}</string>
<key>CFBundleExecutable</key><string>test</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>`)
  const source = path.join(directory, 'main.c')
  writeFileSync(source, 'int main(void) { return 0; }')
  execFileSync('/usr/bin/clang', [source, '-o', path.join(contents, 'MacOS/test')])
  const resource = path.join(contents, 'Resources/test.txt')
  writeFileSync(resource, 'original')
  const sign = (...args) => execFileSync('/usr/bin/codesign', ['--force', '--sign', '-', ...args, app], { stdio: 'pipe' })
  sign()
  checkMacApp(app, identifier)
  assert.throws(() => checkMacApp(app, 'com.zmair.other'), /bundle identifier/)
  writeFileSync(resource, 'modified')
  assert.throws(() => checkMacApp(app, identifier))
  sign('--identifier', 'com.zmair.other')
  assert.throws(() => checkMacApp(app, identifier))
})

test('desktop CSP permits skill and MCP market APIs in production and development', () => {
  const { app: { security } } = JSON.parse(readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'))
  for (const key of ['csp', 'devCsp']) {
    const connect = security[key].split(';')
      .map(directive => directive.trim().split(/\s+/))
      .find(([name]) => name === 'connect-src')
    for (const origin of [
      'https://clawhub.ai',
      'https://registry.modelcontextprotocol.io',
      'https://api.smithery.ai',
      'https://glama.ai',
    ]) {
      assert.ok(connect?.includes(origin), `${key} must allow the market API origin ${origin}`)
    }
    assert.ok(!connect.includes('*') && !connect.includes('https:'), `${key} must not allow arbitrary remote origins`)
  }
})

test('translator overlay disables native drag-region double-click maximization', () => {
  // This builder flag is also the guard used by Tauri's internal_toggle_maximize.
  // Checking the shipped window configuration avoids rebuilding a native app just for this contract.
  const windows = readFileSync(path.join(root, 'src-tauri/src/windows.rs'), 'utf8')
  const translator = windows.match(/pub fn ensure_translator_window\b[\s\S]*?\n}\n/)
  assert.ok(translator, 'Translator window builder must exist')
  assert.match(translator[0], /\.resizable\(false\)/)
})

test('rejects stale MSI versions and mismatched release tags', t => {
  const directory = fixture(t)
  assert.throws(() => checkVersions(directory, 'v0.0.1'), /Release tag/)
  changeJson(directory, 'src-tauri/tauri.conf.json', config => {
    config.bundle.windows.wix = { version: '2.8.2' }
  })
  assert.throws(() => checkVersions(directory), /MSI must inherit/)
})

test('rejects lockfile version drift', t => {
  const directory = fixture(t)
  changeJson(directory, 'package-lock.json', lock => { lock.packages[''].version = '0.0.1' })
  assert.throws(() => checkVersions(directory), /package-lock.json root/)
})

test('packaged resources reject missing licenses, changed files and stale skills', t => {
  const directory = fixture(t)
  const packaged = path.join(directory, 'packaged')
  const { bundle } = JSON.parse(readFileSync(path.join(directory, 'src-tauri/tauri.conf.json'), 'utf8'))
  for (const [source, destination] of Object.entries(bundle.resources)) {
    cpSync(path.resolve(root, 'src-tauri', source), path.resolve(directory, 'src-tauri', source), { recursive: true })
    if (destination !== 'licenses') {
      cpSync(path.resolve(root, 'src-tauri', source), path.join(packaged, destination), { recursive: true })
    }
  }
  assert.throws(() => checkResources(directory, packaged), /ENOENT/)
  cpSync(path.join(root, 'docs/licenses'), path.join(packaged, 'licenses'), { recursive: true })
  assert.ok(checkResources(directory, packaged) > 0)
  writeFileSync(path.join(packaged, 'skills', 'retired-skill.md'), 'old')
  assert.throws(() => checkResources(directory, packaged), /packaged files must match/)
  rmSync(path.join(packaged, 'skills', 'retired-skill.md'))
  const skillFile = readdirSync(path.join(packaged, 'skills'), { recursive: true })
    .find(file => path.basename(file) === 'SKILL.md')
  assert.ok(skillFile, 'Fixture must include a skill')
  writeFileSync(path.join(packaged, 'skills', skillFile), 'changed')
  assert.throws(() => checkResources(directory, packaged), /packaged files must match/)
})
