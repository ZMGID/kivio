#!/usr/bin/env node
// 构建 Apple Intelligence sidecar 二进制：
//   - 仅 macOS 上运行；其他平台直接退出（Tauri 在那些平台不需要这个 sidecar）
//   - swift build -c release 编译 src-tauri/swift/keylingo-ai-helper
//   - 把产物 cp 到 src-tauri/binaries/keylingo-ai-helper-<rustc-target-triple>，让 Tauri 的 externalBin 找到
// 失败不阻塞 dev：调用方用 `|| true` 兜底，allowing developers without macOS 26 to still build the rest.

import { execSync } from 'node:child_process'
import { existsSync, mkdirSync, copyFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')
const SWIFT_PKG = resolve(ROOT, 'src-tauri/swift/keylingo-ai-helper')
const BIN_DIR = resolve(ROOT, 'src-tauri/binaries')

if (process.platform !== 'darwin') {
  console.log('[build-swift-sidecar] non-macOS,跳过 (sidecar 仅 macOS 用)')
  process.exit(0)
}

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
console.log(`[build-swift-sidecar] target triple = ${triple}`)

console.log('[build-swift-sidecar] swift build -c release')
execSync('swift build -c release', { cwd: SWIFT_PKG, stdio: 'inherit' })

const builtPath = resolve(SWIFT_PKG, '.build/release/keylingo-ai-helper')
if (!existsSync(builtPath)) {
  console.error(`[build-swift-sidecar] 编译产物不存在: ${builtPath}`)
  process.exit(1)
}

if (!existsSync(BIN_DIR)) mkdirSync(BIN_DIR, { recursive: true })
const dest = resolve(BIN_DIR, `keylingo-ai-helper-${triple}`)
copyFileSync(builtPath, dest)
execSync(`chmod +x "${dest}"`)
console.log(`[build-swift-sidecar] → ${dest}`)
