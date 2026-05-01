#!/usr/bin/env node
// 一次性脚本：把源 icon (public/icon.png) 处理成 macOS-style squircle 圆角图标，
// 输出到 src-tauri/icons/source-rounded.png，由调用方继续跑 `tauri icon` 生成各平台尺寸。
//
// 修复历史：早期版本把源图 contain 到 824×824 居中放进 1024 透明画布，再做圆角 mask。
// 结果圆角 mask 削的是外围透明 padding，从未切到中央的白底——macOS / Windows 上看起来
// 还是个无圆角的方形白块。现在的逻辑：
//   1. 源图 resize 到 1024×1024（保持原 squircle 设计的视觉比例）
//   2. 底下铺纯白 1024×1024 画布（合成时四角透明会被白色透出）
//   3. 对 1024 边缘应用半径 230 的圆角 mask（≈22.5%，macOS squircle 视觉等效半径）
// 这样圆角才真正切到白底，得到完整的应用图标 squircle。

import { Jimp } from 'jimp'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')

const SRC = resolve(ROOT, 'public/icon.png')
const DST = resolve(ROOT, 'src-tauri/icons/source-rounded.png')

const SIZE = 1024
const RADIUS = 230

async function main() {
  const src = await Jimp.read(SRC)
  src.resize({ w: SIZE, h: SIZE })

  // 纯白底，1024×1024 全填满
  const canvas = new Jimp({ width: SIZE, height: SIZE, color: 0xffffffff })
  canvas.composite(src, 0, 0)

  // 圆角 mask：1024 四角 RADIUS 内、距离圆心 > RADIUS 的像素设为透明，边缘 1px 抗锯齿
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
        canvas.setPixelColor(0x00000000, x, y)
      } else if (d2 > (RADIUS - 1) * (RADIUS - 1)) {
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
