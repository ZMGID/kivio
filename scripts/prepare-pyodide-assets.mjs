#!/usr/bin/env node

import { createHash } from 'node:crypto'
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')
const PYODIDE_DIR = resolve(ROOT, 'node_modules/pyodide')
const CACHE_DIR = resolve(ROOT, '.cache/pyodide')

const CORE_FILES = [
  'pyodide.asm.js',
  'pyodide.asm.wasm',
  'pyodide-lock.json',
  'python_stdlib.zip',
]
const MANIFEST_FILE = 'pyodide-package-manifest.json'

const REQUIRED_PYODIDE_PACKAGES = [
  'numpy',
  'pandas',
  'matplotlib',
  'scipy',
  'sympy',
  'scikit-learn',
  'statsmodels',
  'pillow',
  'micropip',
]

const PYPI_WHEELS = [
  {
    name: 'seaborn',
    version: '0.13.2',
    pyodideDeps: ['numpy', 'pandas', 'matplotlib'],
  },
  {
    name: 'openpyxl',
    version: '3.1.5',
    pyodideDeps: ['pandas', 'et_xmlfile'],
  },
  {
    name: 'et_xmlfile',
    version: '2.0.0',
    pyodideDeps: [],
  },
  {
    name: 'xlrd',
    version: '2.0.1',
    pyodideDeps: ['pandas'],
  },
]

const PYTHON_FONT_FILES = [
  {
    fileName: 'NotoSansCJKsc-Regular.otf',
    family: 'Noto Sans CJK SC',
    url: 'https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@main/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf',
    sha256: '2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b',
  },
]

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'))
}

function sha256(buffer) {
  return createHash('sha256').update(buffer).digest('hex')
}

function fileMatches(path, expectedSha256) {
  if (!existsSync(path)) return false
  if (statSync(path).size <= 0) return false
  if (!expectedSha256) return true
  return sha256(readFileSync(path)) === expectedSha256
}

function copyCoreFiles() {
  for (const fileName of CORE_FILES) {
    const source = join(PYODIDE_DIR, fileName)
    const dest = join(CACHE_DIR, fileName)
    if (!existsSync(source)) {
      throw new Error(`Missing Pyodide core file: ${source}`)
    }
    copyFileSync(source, dest)
  }
}

function collectPyodidePackageClosure(lock, names) {
  const selected = new Set()
  const missing = new Set()
  const stack = [...names]

  while (stack.length > 0) {
    const name = stack.pop()
    if (!name || selected.has(name)) continue
    const pkg = lock.packages[name]
    if (!pkg) {
      missing.add(name)
      continue
    }
    selected.add(name)
    for (const dep of pkg.depends ?? []) {
      if (!selected.has(dep)) stack.push(dep)
    }
  }

  return { selected, missing }
}

async function download(url, label) {
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(`${label} HTTP ${response.status}`)
  }
  return Buffer.from(await response.arrayBuffer())
}

async function downloadPyodidePackage(version, pkg) {
  const fileName = pkg.file_name
  const dest = join(CACHE_DIR, fileName)
  if (fileMatches(dest, pkg.sha256)) {
    return { fileName, status: 'cached' }
  }

  const url = `https://cdn.jsdelivr.net/pyodide/v${version}/full/${encodeURIComponent(fileName)}`
  const bytes = await download(url, fileName)
  const actualHash = sha256(bytes)
  if (actualHash !== pkg.sha256) {
    throw new Error(`Checksum mismatch for ${fileName}: expected ${pkg.sha256}, got ${actualHash}`)
  }
  const tmp = `${dest}.tmp`
  writeFileSync(tmp, bytes)
  renameSync(tmp, dest)
  return { fileName, status: 'downloaded' }
}

async function pypiWheelInfo(name, version) {
  const meta = JSON.parse(
    (await download(`https://pypi.org/pypi/${name}/${version}/json`, `${name} metadata`)).toString('utf8'),
  )
  const wheel = meta.urls.find((item) => (
    typeof item.filename === 'string'
    && item.filename.endsWith('.whl')
    && item.packagetype === 'bdist_wheel'
  ))
  if (!wheel) {
    throw new Error(`No wheel found on PyPI for ${name} ${version}`)
  }
  return {
    fileName: wheel.filename,
    url: wheel.url,
    sha256: wheel.digests?.sha256,
  }
}

async function downloadPypiWheel(spec) {
  const info = await pypiWheelInfo(spec.name, spec.version)
  const dest = join(CACHE_DIR, info.fileName)
  if (fileMatches(dest, info.sha256)) {
    return { fileName: info.fileName, status: 'cached' }
  }
  const bytes = await download(info.url, info.fileName)
  const actualHash = sha256(bytes)
  if (info.sha256 && actualHash !== info.sha256) {
    throw new Error(`Checksum mismatch for ${info.fileName}: expected ${info.sha256}, got ${actualHash}`)
  }
  const tmp = `${dest}.tmp`
  writeFileSync(tmp, bytes)
  renameSync(tmp, dest)
  return { fileName: info.fileName, status: 'downloaded' }
}

async function downloadFontFile(spec) {
  const dest = join(CACHE_DIR, spec.fileName)
  if (fileMatches(dest, spec.sha256)) {
    return { fileName: spec.fileName, status: 'cached' }
  }
  const bytes = await download(spec.url, spec.fileName)
  const actualHash = sha256(bytes)
  if (actualHash !== spec.sha256) {
    throw new Error(`Checksum mismatch for ${spec.fileName}: expected ${spec.sha256}, got ${actualHash}`)
  }
  const tmp = `${dest}.tmp`
  writeFileSync(tmp, bytes)
  renameSync(tmp, dest)
  return { fileName: spec.fileName, status: 'downloaded' }
}

async function main() {
  if (!existsSync(PYODIDE_DIR)) {
    throw new Error('node_modules/pyodide is missing. Run npm install first.')
  }
  mkdirSync(CACHE_DIR, { recursive: true })
  copyCoreFiles()

  const pyodidePackage = readJson(join(PYODIDE_DIR, 'package.json'))
  const lock = readJson(join(PYODIDE_DIR, 'pyodide-lock.json'))
  const requestedPyodidePackages = [
    ...REQUIRED_PYODIDE_PACKAGES,
    ...PYPI_WHEELS.flatMap((spec) => spec.pyodideDeps),
  ]
  const { selected, missing } = collectPyodidePackageClosure(lock, requestedPyodidePackages)
  if (missing.size > 0) {
    console.warn(`[prepare-pyodide-assets] Missing Pyodide package(s): ${[...missing].sort().join(', ')}`)
  }

  const downloaded = []
  for (const name of [...selected].sort()) {
    downloaded.push(await downloadPyodidePackage(pyodidePackage.version, lock.packages[name]))
  }
  const manifest = {
    pyodideVersion: pyodidePackage.version,
    coreFiles: CORE_FILES,
    requiredPyodidePackages: [...selected].sort(),
    fontFiles: PYTHON_FONT_FILES.map(({ fileName, family, sha256 }) => ({ fileName, family, sha256 })),
    pypiWheels: {},
  }
  for (const wheel of PYPI_WHEELS) {
    const result = await downloadPypiWheel(wheel)
    downloaded.push(result)
    manifest.pypiWheels[wheel.name] = {
      version: wheel.version,
      fileName: result.fileName,
      pyodideDeps: wheel.pyodideDeps,
    }
  }
  for (const font of PYTHON_FONT_FILES) {
    downloaded.push(await downloadFontFile(font))
  }

  writeFileSync(join(CACHE_DIR, MANIFEST_FILE), `${JSON.stringify(manifest, null, 2)}\n`)
  rmSync(join(CACHE_DIR, '.download-failed'), { force: true })
  const downloadedCount = downloaded.filter((item) => item.status === 'downloaded').length
  const cachedCount = downloaded.filter((item) => item.status === 'cached').length
  console.log(
    `[prepare-pyodide-assets] Ready: ${CORE_FILES.length} core files, ${downloaded.length} package/font files (${downloadedCount} downloaded, ${cachedCount} cached).`,
  )
}

main().catch((err) => {
  mkdirSync(CACHE_DIR, { recursive: true })
  writeFileSync(join(CACHE_DIR, '.download-failed'), String(err?.stack || err?.message || err))
  console.error(`[prepare-pyodide-assets] ${err?.message || err}`)
  process.exit(1)
})
