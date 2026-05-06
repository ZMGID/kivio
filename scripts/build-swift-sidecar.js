#!/usr/bin/env node
// 构建 Swift sidecar 二进制：
//   - kivio-ai-helper: Apple Foundation Models 文本/流式调用，只有 macOS 26+ 才可用。
//   - kivio-ocr-helper: Apple Vision OCR，独立于 Apple Intelligence。
//   - 非 macOS(Windows/Linux): 写空 stub 占位,让 Tauri 的 externalBin 文件存在校验通过。
//     运行时对应 client 会按平台直接禁用,不会 spawn 这些 stub。
//     这是 Tauri externalBin 的设计限制——配置是全平台的,无法仅 macOS 启用。
// dev 可用 `|| true` 兜底；正式 build 必须让必需 helper 构建失败直接中止。

import { execSync } from 'node:child_process'
import { existsSync, mkdirSync, copyFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')
const BIN_DIR = resolve(ROOT, 'src-tauri/binaries')
const HELPERS = [
  {
    name: 'kivio-ai-helper',
    pkg: resolve(ROOT, 'src-tauri/swift/kivio-ai-helper'),
    optional: true,
  },
  {
    name: 'kivio-ocr-helper',
    pkg: resolve(ROOT, 'src-tauri/swift/kivio-ocr-helper'),
    optional: false,
  },
]

function detectRustTriple() {
  try {
    const out = execSync('rustc -vV', { encoding: 'utf8' })
    const m = out.match(/host:\s*(\S+)/)
    if (!m) throw new Error('rustc -vV 输出里没找到 host')
    return m[1]
  } catch (err) {
    console.error('[build-swift-sidecar] 探测 rustc target triple 失败:', err.message)
    process.exit(1)
  }
}

const triple = detectRustTriple()
const exeSuffix = process.platform === 'win32' ? '.exe' : ''

if (!existsSync(BIN_DIR)) mkdirSync(BIN_DIR, { recursive: true })

function makeDestName(name) {
  return `${name}-${triple}${exeSuffix}`
}

function makeExecutable(file) {
  if (process.platform !== 'win32') {
    execSync(`chmod +x "${file}"`)
  }
}

function ensureStub(name) {
  const dest = resolve(BIN_DIR, makeDestName(name))
  if (!existsSync(dest)) {
    writeFileSync(dest, '')
    makeExecutable(dest)
    console.log(`[build-swift-sidecar] 写空 stub → ${dest}`)
  } else {
    console.log(`[build-swift-sidecar] stub 已存在 → ${dest}`)
  }
}

if (process.platform !== 'darwin') {
  for (const helper of HELPERS) ensureStub(helper.name)
  process.exit(0)
}

console.log(`[build-swift-sidecar] target triple = ${triple}`)

let failedRequired = false
for (const helper of HELPERS) {
  const dest = resolve(BIN_DIR, makeDestName(helper.name))
  console.log(`[build-swift-sidecar] ${helper.name}: swift build -c release`)
  try {
    execSync('swift build -c release', { cwd: helper.pkg, stdio: 'inherit' })
    const builtPath = resolve(helper.pkg, `.build/release/${helper.name}`)
    if (!existsSync(builtPath)) {
      throw new Error(`编译产物不存在: ${builtPath}`)
    }
    copyFileSync(builtPath, dest)
    makeExecutable(dest)
    console.log(`[build-swift-sidecar] ${helper.name} → ${dest}`)
  } catch (err) {
    console.error(`[build-swift-sidecar] ${helper.name} 构建失败: ${err.message}`)
    if (!helper.optional) {
      failedRequired = true
      continue
    }
    ensureStub(helper.name)
  }
}

if (failedRequired) {
  process.exit(1)
}
