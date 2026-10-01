import assert from 'node:assert/strict'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { checkResources, checkVersions } from './check-desktop-package.mjs'

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
  writeFileSync(path.join(packaged, 'skills', 'pdf', 'SKILL.md'), 'changed')
  assert.throws(() => checkResources(directory, packaged), /packaged files must match/)
})
