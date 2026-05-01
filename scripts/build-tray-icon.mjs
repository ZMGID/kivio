#!/usr/bin/env node
// 一次性脚本：把用户提供的纯黑 logo（白底）处理成 macOS template-image 风格的 tray icon。
//
// macOS menubar icon 标准（HIG）：
//   - Template image：纯黑色 + 透明背景，系统按 light/dark 主题自动反色
//   - 内容尺寸 18x18 in 22x22 canvas（≈18% padding，避免内容贴边）
//   - HiDPI 4x = 72x72 content in 88x88 canvas
//
// 处理：
//   1. 读源图，luma>200 的像素当作"白底"设为透明，其余像素压成纯黑
//   2. autocrop 去除边缘空白
//   3. contain 缩到 72x72 content
//   4. 居中放到 88x88 透明 canvas

import { Jimp } from 'jimp'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')

const SRC = process.argv[2] || resolve(process.env.HOME, 'Downloads/抠图 纯黑.png')
const DST = resolve(ROOT, 'src-tauri/icons/tray-icon.png')

const CANVAS = 88   // 4x of 22 (HiDPI menubar canvas)
const CONTENT = 72  // 4x of 18 (HIG 推荐内容区)

async function main() {
  const img = await Jimp.read(SRC)

  // 二值化：白底（亮度高）→ 透明；其它 → 纯黑不透明
  img.scan(0, 0, img.bitmap.width, img.bitmap.height, function (_x, _y, idx) {
    const r = this.bitmap.data[idx]
    const g = this.bitmap.data[idx + 1]
    const b = this.bitmap.data[idx + 2]
    const luma = (r + g + b) / 3
    if (luma > 200) {
      this.bitmap.data[idx] = 0
      this.bitmap.data[idx + 1] = 0
      this.bitmap.data[idx + 2] = 0
      this.bitmap.data[idx + 3] = 0
    } else {
      this.bitmap.data[idx] = 0
      this.bitmap.data[idx + 1] = 0
      this.bitmap.data[idx + 2] = 0
      this.bitmap.data[idx + 3] = 255
    }
  })

  img.autocrop()
  img.contain({ w: CONTENT, h: CONTENT })

  const canvas = new Jimp({ width: CANVAS, height: CANVAS, color: 0x00000000 })
  const ox = Math.floor((CANVAS - img.bitmap.width) / 2)
  const oy = Math.floor((CANVAS - img.bitmap.height) / 2)
  canvas.composite(img, ox, oy)

  await canvas.write(DST)
  console.log(`[tray-icon] ${img.bitmap.width}x${img.bitmap.height} content centered in ${CANVAS}x${CANVAS} → ${DST}`)
}

main().catch((err) => {
  console.error('[tray-icon] failed:', err)
  process.exit(1)
})
