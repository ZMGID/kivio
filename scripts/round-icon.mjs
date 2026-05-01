#!/usr/bin/env node
// 一次性脚本：把源 icon (1254x1254 方形) 处理成 macOS-style squircle 圆角图标，
// 输出到 src-tauri/icons/source-rounded.png，由调用方继续跑 `tauri icon` 生成各平台尺寸。
//
// 处理：
//   1. 把源图缩到 824x824（macOS HIG 推荐：1024 容器内留 100px padding）
//   2. 居中放到 1024x1024 透明 canvas
//   3. 应用圆角矩形 mask（半径 230px ≈ 22.5%，接近 macOS squircle）
//
// 为什么不直接做 squircle (superellipse)：圆角矩形够用，肉眼几乎分辨不出差别，
// 实现简单。如果以后想精确 squircle 再说。

import { Jimp } from 'jimp'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')

const SRC = resolve(ROOT, 'public/icon.png')             // 当前的 1254x1254 方形图
const DST = resolve(ROOT, 'src-tauri/icons/source-rounded.png')

const SIZE = 1024
const PADDING = 100  // 内容区四边各留 100px
const RADIUS = 230   // ~22.5% of 1024，macOS squircle 视觉等效半径

async function main() {
  const src = await Jimp.read(SRC)
  src.contain({ w: SIZE - PADDING * 2, h: SIZE - PADDING * 2 })

  const canvas = new Jimp({ width: SIZE, height: SIZE, color: 0x00000000 })
  // 居中合成
  const offsetX = Math.floor((SIZE - src.bitmap.width) / 2)
  const offsetY = Math.floor((SIZE - src.bitmap.height) / 2)
  canvas.composite(src, offsetX, offsetY)

  // 圆角 mask：4 个角 radius 内的像素，距离圆心 > radius 的设为透明
  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      let cx = -1, cy = -1
      if (x < RADIUS && y < RADIUS) { cx = RADIUS; cy = RADIUS }
      else if (x >= SIZE - RADIUS && y < RADIUS) { cx = SIZE - RADIUS - 1; cy = RADIUS }
      else if (x < RADIUS && y >= SIZE - RADIUS) { cx = RADIUS; cy = SIZE - RADIUS - 1 }
      else if (x >= SIZE - RADIUS && y >= SIZE - RADIUS) { cx = SIZE - RADIUS - 1; cy = SIZE - RADIUS - 1 }
      if (cx < 0) continue
      const dx = x - cx, dy = y - cy
      const d2 = dx * dx + dy * dy
      const r2 = RADIUS * RADIUS
      if (d2 > r2) {
        // 完全透明
        canvas.setPixelColor(0x00000000, x, y)
      } else if (d2 > (RADIUS - 1) * (RADIUS - 1)) {
        // 边缘 1px 抗锯齿：alpha 按距离线性插值
        const d = Math.sqrt(d2)
        const alpha = Math.max(0, Math.min(1, RADIUS - d))
        const px = canvas.getPixelColor(x, y)
        const r = (px >>> 24) & 0xff
        const g = (px >>> 16) & 0xff
        const b = (px >>> 8) & 0xff
        const a = px & 0xff
        const newA = Math.round(a * alpha)
        canvas.setPixelColor(((r << 24) | (g << 16) | (b << 8) | newA) >>> 0, x, y)
      }
    }
  }

  await canvas.write(DST)
  console.log(`[round-icon] → ${DST}`)
}

main().catch((err) => {
  console.error('[round-icon] failed:', err)
  process.exit(1)
})
