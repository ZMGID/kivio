#!/usr/bin/env node
// 构建 Apple Intelligence sidecar 二进制：
//   - macOS: swift build -c release 编译 src-tauri/swift/keylingo-ai-helper,产物 cp 到
//     src-tauri/binaries/keylingo-ai-helper-<rustc-target-triple>(Tauri externalBin 命名规则)
//   - 非 macOS(Windows/Linux): 写一个空 stub 占位,让 Tauri 的 externalBin 文件存在校验通过。
//     运行时 AppleIntelligenceClient::new 尝试 spawn 会失败,available=false,UI 自动隐藏 Apple chip。
//     这是 Tauri externalBin 的设计限制——配置是全平台的,无法仅 macOS 启用。
// 失败不阻塞 dev：调用方用 `|| true` 兜底。

import { execSync } from 'node:child_process'
import { existsSync, mkdirSync, copyFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')
const SWIFT_PKG = resolve(ROOT, 'src-tauri/swift/keylingo-ai-helper')
const BIN_DIR = resolve(ROOT, 'src-tauri/binaries')

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
const destName = `keylingo-ai-helper-${triple}${exeSuffix}`
const dest = resolve(BIN_DIR, destName)

if (!existsSync(BIN_DIR)) mkdirSync(BIN_DIR, { recursive: true })

if (process.platform !== 'darwin') {
  // 非 macOS:写空 stub 让 Tauri externalBin 校验通过。运行时 sidecar 不会真正被 spawn 使用。
  if (!existsSync(dest)) {
    writeFileSync(dest, '')
    console.log(`[build-swift-sidecar] non-macOS: 写空 stub → ${dest}`)
  } else {
    console.log(`[build-swift-sidecar] non-macOS: stub 已存在 → ${dest}`)
  }
  process.exit(0)
}

console.log(`[build-swift-sidecar] target triple = ${triple}`)
console.log('[build-swift-sidecar] swift build -c release')
execSync('swift build -c release', { cwd: SWIFT_PKG, stdio: 'inherit' })

const builtPath = resolve(SWIFT_PKG, '.build/release/keylingo-ai-helper')
if (!existsSync(builtPath)) {
  console.error(`[build-swift-sidecar] 编译产物不存在: ${builtPath}`)
  process.exit(1)
}

copyFileSync(builtPath, dest)
execSync(`chmod +x "${dest}"`)
console.log(`[build-swift-sidecar] → ${dest}`)
