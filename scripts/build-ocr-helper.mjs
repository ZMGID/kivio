import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const projectRoot = process.cwd()
const src = path.join(projectRoot, 'native', 'ocr', 'keylingo_ocr.swift')
const outDir = path.join(projectRoot, 'resources', 'ocr')
const out = path.join(outDir, 'keylingo-ocr')
const cacheDir = path.join(projectRoot, '.cache', 'swift')
const clangCacheDir = path.join(cacheDir, 'clang-modules')

if (!fs.existsSync(src)) {
  console.error(`OCR helper source not found: ${src}`)
  process.exit(1)
}

fs.mkdirSync(outDir, { recursive: true })
fs.mkdirSync(clangCacheDir, { recursive: true })

try {
  execFileSync(
    'xcrun',
    [
      '--sdk',
      'macosx',
      'swiftc',
      '-O',
      src,
      '-o',
      out,
      '-module-cache-path',
      cacheDir,
      '-Xcc',
      `-fmodules-cache-path=${clangCacheDir}`,
    ],
    { stdio: 'inherit' },
  )
  fs.chmodSync(out, 0o755)
  console.log(`Built OCR helper: ${out}`)
} catch (err) {
  console.error('Failed to build OCR helper. Make sure Xcode Command Line Tools are installed (swiftc available).')
  throw err
}
