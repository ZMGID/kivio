import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { lstatSync, readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const json = file => JSON.parse(readFileSync(file, 'utf8'))

export function checkVersions(root, expectedVersion) {
  const config = json(path.join(root, 'src-tauri/tauri.conf.json'))
  const pkg = json(path.join(root, 'package.json'))
  const lock = json(path.join(root, 'package-lock.json'))
  const cargo = readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8')
    .split(/^\[package\]\s*$/m)[1]?.split(/^\[/m)[0]
  const cargoLock = readFileSync(path.join(root, 'src-tauri/Cargo.lock'), 'utf8')
    .split(/^\[\[package\]\]\s*$/m).find(section => /^name = "kivio"$/m.test(section))
  for (const [name, version] of Object.entries({
    'package.json': pkg.version,
    'package-lock.json': lock.version,
    'package-lock.json root': lock.packages?.['']?.version,
    'Cargo.toml': cargo?.match(/^version\s*=\s*"([^"]+)"/m)?.[1],
    'Cargo.lock': cargoLock?.match(/^version\s*=\s*"([^"]+)"/m)?.[1],
  })) {
    assert.equal(version, config.version, `${name} must match tauri.conf.json`)
  }
  assert.match(config.version, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/, 'Invalid release version')
  if (expectedVersion !== undefined) {
    assert.equal(expectedVersion.replace(/^v/, ''), config.version, 'Release tag must match app version')
  }
  assert.equal(config.bundle.windows.wix?.version, undefined, 'MSI must inherit the app version')
  assert.ok(config.bundle.macOS?.signingIdentity?.trim(), 'macOS signing identity must be explicit')
  return config
}

export function checkMacApp(appDirectory, identifier) {
  assert.ok(lstatSync(path.join(appDirectory, 'Contents/_CodeSignature/CodeResources')).isFile(),
    'macOS application bundle signature is missing')
  execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', appDirectory], { stdio: 'pipe' })
  const bundleId = execFileSync('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleIdentifier',
    path.join(appDirectory, 'Contents/Info.plist')], { encoding: 'utf8' }).trim()
  assert.equal(bundleId, identifier, 'macOS bundle identifier must match the app configuration')
  execFileSync('/usr/bin/codesign', ['--verify', '-R', `identifier "${identifier}"`, appDirectory], { stdio: 'pipe' })
}

function manifest(directory, prefix = '') {
  const result = {}
  for (const name of readdirSync(directory).sort()) {
    const relative = prefix ? `${prefix}/${name}` : name
    const file = path.join(directory, name)
    const stat = lstatSync(file)
    assert.ok(!stat.isSymbolicLink(), `Bundled resources must not contain symlinks: ${file}`)
    if (stat.isDirectory()) Object.assign(result, manifest(file, relative))
    else {
      assert.ok(stat.isFile(), `Not a resource file: ${file}`)
      result[relative] = createHash('sha256').update(readFileSync(file)).digest('hex')
    }
  }
  return result
}

export function checkResources(root, resourceDirectory) {
  const { bundle } = json(path.join(root, 'src-tauri/tauri.conf.json'))
  assert.ok(bundle.resources && !Array.isArray(bundle.resources), 'Expected directory resource mappings')
  assert.equal(bundle.resources['resources/skills'], 'skills', 'Bundled skills mapping missing')
  assert.equal(bundle.resources['../docs/licenses'], 'licenses', 'Bundled licenses mapping missing')
  let count = 0
  for (const [source, destination] of Object.entries(bundle.resources)) {
    assert.ok(destination && !path.isAbsolute(destination)
      && !destination.split(/[\\/]/).includes('..'), `Unsafe resource destination: ${destination}`)
    const sourceManifest = manifest(path.resolve(root, 'src-tauri', source))
    assert.ok(Object.keys(sourceManifest).length > 0, `Empty resource source: ${source}`)
    if (resourceDirectory) {
      assert.deepEqual(manifest(path.join(resourceDirectory, destination)), sourceManifest,
        `${destination}: packaged files must match sources (no missing, changed or retired files)`)
    }
    count += Object.keys(sourceManifest).length
  }
  return count
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    let resourceDirectory
    let expectedVersion
    let appDirectory
    const args = process.argv.slice(2)
    while (args.length) {
      const flag = args.shift()
      assert.ok(['--resources-dir', '--version', '--macos-app'].includes(flag) && args.length, `Invalid argument: ${flag}`)
      const value = args.shift()
      if (flag === '--resources-dir') resourceDirectory = path.resolve(value)
      else if (flag === '--macos-app') appDirectory = path.resolve(value)
      else expectedVersion = value
    }
    const config = checkVersions(repoRoot, expectedVersion)
    const count = checkResources(repoRoot, resourceDirectory)
    if (appDirectory) checkMacApp(appDirectory, config.identifier)
    console.log(`Desktop package checked: v${config.version}, ${count} resource files${resourceDirectory ? ' (packaged contents match)' : ''}.`)
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}
