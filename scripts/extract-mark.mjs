#!/usr/bin/env node
// 一次性脚本：把"假抠图"PNG（实际是浅灰底 + 黑 logo）转成真正的透明背景 + 黑色 logo。
//
// 处理：每个像素 luma >= 230 → alpha=0；luma <= 50 → alpha=255；中间线性插值。
// 输出 RGB 全部固定为 (0,0,0)，由 alpha 通道决定可见度——这样 dark:invert 在深色模式可以把
// 黑色翻转为白色，且边缘抗锯齿天然保留。
//
// 用法: node scripts/extract-mark.mjs <源PNG> [目标PNG]
// 默认源 = ~/Downloads/抠图 纯黑.png，默认目标 = public/logo-mark.png

import { Jimp } from 'jimp'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import os from 'node:os'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')

const SRC = process.argv[2] || resolve(os.homedir(), 'Downloads', '抠图 纯黑.png')
const DST = process.argv[3] || resolve(ROOT, 'public/logo-mark.png')

// 亮度阈值（0..255）
const BLACK = 50    // <= 此值视为完全黑
const WHITE = 230   // >= 此值视为完全白（透明）

async function main() {
  const img = await Jimp.read(SRC)
  const { width, height } = img.bitmap
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const px = img.getPixelColor(x, y)
      const r = (px >>> 24) & 0xff
      const g = (px >>> 16) & 0xff
      const b = (px >>> 8) & 0xff
      // ITU-R BT.601 luma
      const luma = 0.299 * r + 0.587 * g + 0.114 * b
      let alpha
      if (luma <= BLACK) alpha = 255
      else if (luma >= WHITE) alpha = 0
      else alpha = Math.round(((WHITE - luma) / (WHITE - BLACK)) * 255)
      const newPx = ((0 << 24) | (0 << 16) | (0 << 8) | alpha) >>> 0
      img.setPixelColor(newPx, x, y)
    }
  }
  await img.write(DST)
  console.log(`[extract-mark] ${SRC} → ${DST} (${width}x${height})`)
}

main().catch((err) => {
  console.error('[extract-mark] failed:', err)
  process.exit(1)
})
