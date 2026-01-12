import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const projectRoot = process.cwd()
if (process.platform === 'darwin') {
  const src = path.join(projectRoot, 'native', 'ocr', 'keylingo_ocr.swift')
  const outDir = path.join(projectRoot, 'resources', 'ocr')
  const out = path.join(outDir, 'keylingo-ocr')
  const cacheDir = path.join(projectRoot, '.cache', 'swift')
  const clangCacheDir = path.join(cacheDir, 'clang-modules')

  if (!fs.existsSync(src)) {
    console.error(`OCR helper source not found: ${src}`)
    process.exit(1)
  }

  // Create output dir
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
    console.log(`Built MacOS OCR helper: ${out}`)
  } catch (err) {
    console.error('Failed to build OCR helper. Make sure Xcode Command Line Tools are installed.')
    throw err
  }
} else if (process.platform === 'win32') {
  const tpl = path.join(projectRoot, 'native', 'win-ocr', 'KeyLingo.Ocr.csproj')
  if (!fs.existsSync(tpl)) {
    console.warn('Windows OCR source not found, skipping build.')
  } else {
    try {
      console.log('Building Windows OCR helper...')
      execFileSync('dotnet', ['build', tpl, '--configuration', 'Release'], { stdio: 'inherit' })

      // 复制编译结果到 resources 目录
      const winOutDir = path.join(projectRoot, 'resources', 'ocr')
      const binSource = path.join(projectRoot, 'native', 'win-ocr', 'bin', 'Release', 'net6.0-windows10.0.19041.0', 'keylingo-ocr.exe')
      const binDest = path.join(winOutDir, 'keylingo-ocr.exe')

      fs.mkdirSync(winOutDir, { recursive: true })

      if (fs.existsSync(binSource)) {
        fs.copyFileSync(binSource, binDest)
        console.log(`Copied Windows OCR binary to: ${binDest}`)
      } else {
        console.warn(`Warning: Built binary not found at ${binSource}`)
      }
    } catch (e) {
      console.error('Failed to build Windows OCR. Ensure dotnet SDK is installed.')
      // Non-fatal if just cross-compiling or dev setup incomplete
    }
  }
} else {
  console.log('Skipping OCR build for platform:', process.platform)
}
