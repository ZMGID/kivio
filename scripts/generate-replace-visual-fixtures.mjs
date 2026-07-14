import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { Buffer } from 'node:buffer'
import { deflateSync } from 'node:zlib'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const fixturesRoot = path.join(root, 'tests/fixtures/replace-translation/v1')
const checkOnly = process.argv.includes('--check')

// Each `lines` entry is the deterministic ground-truth OCR leaf for one drawn
// text line, taken directly from the coordinates the paint functions use. The
// leaf `quad`, the leaf id, and the expected render-slot anchor are all derived
// from the SAME box, so `leaves.json` (the pipeline's OCR input) and
// `expected_geometry.json` (the desired render anchors) are mutually consistent
// by construction — the anchors are ground truth, never copied from the layout
// pipeline itself.
const fixtures = {
  'menu-list': {
    width: 360,
    height: 220,
    paint: paintMenu,
    lines: [
      { id: 'r0000', text: 'Open repository', x: 78, y: 51, width: 116, height: 17, column: 'menu' },
      { id: 'r0001', text: 'Replace translation', x: 78, y: 101, width: 116, height: 17, column: 'menu' },
      { id: 'r0002', text: 'Download offline pack', x: 78, y: 151, width: 116, height: 17, column: 'menu' },
    ],
  },
  'document-paragraph': {
    width: 420,
    height: 260,
    paint: paintDocument,
    lines: [
      { id: 'r0000', text: 'Replace translation architecture', x: 52, y: 48, width: 210, height: 10, column: 'main' },
      {
        id: 'r0001',
        text: 'Translations may merge context but the drawn region keeps each line top anchor',
        x: 52,
        y: 92,
        width: 292,
        height: 31,
        column: 'main',
      },
    ],
  },
  'table-grid': {
    width: 420,
    height: 240,
    paint: paintTable,
    lines: [
      { id: 'r0000', text: 'Name', x: 38, y: 38, width: 110, height: 10, column: 'left' },
      { id: 'r0001', text: 'Status', x: 228, y: 38, width: 126, height: 10, column: 'right' },
      { id: 'r0002', text: 'Local model', x: 38, y: 88, width: 110, height: 10, column: 'left' },
      { id: 'r0003', text: 'Ready', x: 228, y: 88, width: 126, height: 10, column: 'right' },
    ],
  },
  'code-badge': {
    width: 360,
    height: 180,
    paint: paintCodeBadge,
    lines: [
      { id: 'r0000', text: 'run npm test', x: 68, y: 76, width: 158, height: 10, column: 'code' },
    ],
  },
  'photo-gradient': {
    width: 420,
    height: 260,
    paint: paintPhoto,
    lines: [
      { id: 'r0000', text: 'Photo grade replacement', x: 86, y: 88, width: 244, height: 17, column: 'sign', rotationDeg: 0 },
    ],
  },
}

for (const [id, fixture] of Object.entries(fixtures)) {
  const source = rgbaImage(fixture.width, fixture.height, [245, 247, 250, 255])
  const protection = rgbaImage(fixture.width, fixture.height, [0, 0, 0, 255])
  fixture.paint(source, protection, fixture.width, fixture.height)
  const dir = path.join(fixturesRoot, id)
  const files = [
    ['source.png', encodePng(fixture.width, fixture.height, source)],
    ['expected_protection.png', encodePng(fixture.width, fixture.height, protection)],
    ['leaves.json', jsonFile(leavesFor(fixture.lines))],
    ['expected_geometry.json', jsonFile(geometryFor(fixture.lines))],
  ]
  for (const [name, contents] of files) {
    const target = path.join(dir, name)
    if (checkOnly) {
      if (!fs.existsSync(target) || !fs.readFileSync(target).equals(contents)) {
        throw new Error(`${path.relative(root, target)} is missing or stale; run npm run fixtures:replace-visual`)
      }
    } else {
      fs.mkdirSync(dir, { recursive: true })
      fs.writeFileSync(target, contents)
    }
  }
}

if (!checkOnly) console.log(`generated ${Object.keys(fixtures).length} replace-translation visual fixtures`)

function jsonFile(value) {
  return Buffer.from(`${JSON.stringify(value, null, 2)}\n`)
}

function leavesFor(lines) {
  return {
    leaves: lines.map((line, index) => ({
      id: line.id,
      text: line.text,
      quad: [
        [line.x, line.y],
        [line.x + line.width, line.y],
        [line.x + line.width, line.y + line.height],
        [line.x, line.y + line.height],
      ],
      readingOrder: index,
    })),
  }
}

function geometryFor(lines) {
  return {
    slots: lines.map(line => ({
      id: line.id,
      bounds: { x: line.x, y: line.y, width: line.width, height: line.height },
      anchor: { x: line.x, y: line.y },
      column: line.column,
      ...(line.rotationDeg === undefined ? {} : { rotationDeg: line.rotationDeg }),
    })),
  }
}

function rgbaImage(width, height, color) {
  const data = new Uint8Array(width * height * 4)
  for (let index = 0; index < width * height; index += 1) data.set(color, index * 4)
  return data
}

function setPixel(image, width, x, y, color) {
  if (x < 0 || y < 0 || x >= width || y * width * 4 >= image.length) return
  image.set(color, (y * width + x) * 4)
}

function fillRect(image, width, x, y, rectWidth, rectHeight, color) {
  for (let py = y; py < y + rectHeight; py += 1) {
    for (let px = x; px < x + rectWidth; px += 1) setPixel(image, width, px, py, color)
  }
}

function strokeRect(image, width, x, y, rectWidth, rectHeight, color, thickness = 1) {
  fillRect(image, width, x, y, rectWidth, thickness, color)
  fillRect(image, width, x, y + rectHeight - thickness, rectWidth, thickness, color)
  fillRect(image, width, x, y, thickness, rectHeight, color)
  fillRect(image, width, x + rectWidth - thickness, y, thickness, rectHeight, color)
}

function textBars(image, width, x, y, lengths, color = [31, 41, 55, 255]) {
  lengths.forEach((length, index) => fillRect(image, width, x, y + index * 7, length, 3, color))
}

function protectRect(mask, width, x, y, rectWidth, rectHeight) {
  fillRect(mask, width, x, y, rectWidth, rectHeight, [255, 255, 255, 255])
}

function paintMenu(source, protection, width) {
  fillRect(source, width, 20, 18, 320, 184, [255, 255, 255, 255])
  strokeRect(source, width, 20, 18, 320, 184, [203, 213, 225, 255], 2)
  protectRect(protection, width, 20, 18, 320, 2)
  protectRect(protection, width, 20, 200, 320, 2)
  for (const [index, y] of [48, 98, 148].entries()) {
    fillRect(source, width, 36, y, 24, 24, [251, 146, 60, 255])
    strokeRect(source, width, 36, y, 24, 24, [234, 88, 12, 255], 2)
    textBars(source, width, 78, y + 3, [116, 84, 56])
    protectRect(protection, width, 36, y, 24, 24)
    if (index < 2) {
      fillRect(source, width, 32, y + 37, 296, 1, [226, 232, 240, 255])
      protectRect(protection, width, 32, y + 37, 296, 1)
    }
  }
}

function paintDocument(source, protection, width) {
  fillRect(source, width, 28, 18, 364, 224, [255, 255, 255, 255])
  strokeRect(source, width, 28, 18, 364, 224, [148, 163, 184, 255], 2)
  textBars(source, width, 52, 48, [210, 190], [15, 23, 42, 255])
  textBars(source, width, 52, 92, [292, 276, 245, 286, 196])
  protectRect(protection, width, 28, 18, 364, 2)
  protectRect(protection, width, 28, 240, 364, 2)
}

function paintTable(source, protection, width) {
  fillRect(source, width, 20, 20, 380, 200, [255, 255, 255, 255])
  for (const x of [20, 210, 400]) {
    fillRect(source, width, x, 20, 2, 200, [71, 85, 105, 255])
    protectRect(protection, width, x, 20, 2, 200)
  }
  for (const y of [20, 70, 120, 170, 220]) {
    fillRect(source, width, 20, y, 380, 2, [71, 85, 105, 255])
    protectRect(protection, width, 20, y, 380, 2)
  }
  // Only the first two row bands carry text (a header row and one data row), so
  // the fixture stays a clean 2x2 table whose four cells match leaves.json /
  // expected_geometry.json / translations.json one-for-one.
  for (const y of [38, 88]) {
    textBars(source, width, 38, y, [110, 76])
    textBars(source, width, 228, y, [126, 92])
  }
}

function paintCodeBadge(source, protection, width) {
  fillRect(source, width, 28, 34, 304, 112, [255, 255, 255, 255])
  fillRect(source, width, 52, 62, 220, 42, [226, 232, 240, 255])
  strokeRect(source, width, 52, 62, 220, 42, [148, 163, 184, 255], 2)
  textBars(source, width, 68, 76, [158, 122])
  fillRect(source, width, 288, 70, 24, 24, [251, 146, 60, 255])
  protectRect(protection, width, 52, 62, 220, 2)
  protectRect(protection, width, 52, 102, 220, 2)
  protectRect(protection, width, 288, 70, 24, 24)
}

function paintPhoto(source, protection, width, height) {
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const noise = ((x * 17 + y * 29) % 23) - 11
      setPixel(source, width, x, y, [
        Math.max(0, Math.min(255, 38 + x * 0.35 + noise)),
        Math.max(0, Math.min(255, 82 + y * 0.38 + noise)),
        Math.max(0, Math.min(255, 118 + x * 0.18 + noise)),
        255,
      ])
    }
  }
  fillRect(source, width, 54, 58, 312, 96, [20, 28, 36, 160])
  textBars(source, width, 86, 88, [244, 208, 176], [248, 250, 252, 255])
  protectRect(protection, width, 0, 0, width, 12)
  protectRect(protection, width, 0, height - 12, width, 12)
}

function encodePng(width, height, rgba) {
  const stride = width * 4
  const raw = Buffer.alloc((stride + 1) * height)
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0
    Buffer.from(rgba.buffer, rgba.byteOffset + y * stride, stride).copy(raw, y * (stride + 1) + 1)
  }
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', Buffer.from([
      width >>> 24, width >>> 16, width >>> 8, width,
      height >>> 24, height >>> 16, height >>> 8, height,
      8, 6, 0, 0, 0,
    ])),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

function chunk(type, data) {
  const typeBuffer = Buffer.from(type)
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length)
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])))
  return Buffer.concat([length, typeBuffer, data, crc])
}

function crc32(buffer) {
  let crc = 0xffffffff
  for (const value of buffer) {
    crc ^= value
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0)
  }
  return (crc ^ 0xffffffff) >>> 0
}
