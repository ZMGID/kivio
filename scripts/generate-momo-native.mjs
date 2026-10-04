import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'src/chat/kivioBlobShapes.ts')
const outputPath = path.join(root, 'src-tauri/src/desktop_pet/momo_geometry.inc.rs')
const outputRel = path.relative(root, outputPath)
const require = createRequire(import.meta.url)
const ts = require('typescript')

// Enum order in shapes.rs. Discriminants are not used as indexes; the match
// arms name variants, and this list only rejects a drifted TypeScript export.
const BODIES = ['circle', 'squircle', 'cloud', 'bubble', 'puddle', 'burst', 'pebble', 'bean']
const FACES = [
  'neutral',
  'dots',
  'wide',
  'tiny',
  'lines',
  'focus',
  'worry',
  'smirk',
  'happy',
  'content',
  'sleepy',
  'dizzy',
  'sparkle',
  'lookUp',
  'hmm',
  'peek',
  'flat',
]
const BODY_LEN = 72
const EYE_LEN = 48

const args = process.argv.slice(2)
const unknown = args.filter(arg => arg !== '--check')
if (unknown.length) throw new Error(`unknown argument: ${unknown.join(' ')}`)
const check = args.includes('--check')

if (typeof ts.transpileModule !== 'function') {
  throw new Error('installed typescript has no transpileModule')
}

// The shapes module is import-free, so the transpiled ESM can be loaded from a
// data URL with no resolver. Coordinates stay in the 240 viewBox; the sampler
// applies SIZE/240 when it builds a Visual.
const source = readFileSync(sourcePath, 'utf8')
const transpiled = ts.transpileModule(source, {
  fileName: sourcePath,
  reportDiagnostics: true,
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
    sourceMap: false,
    inlineSourceMap: false,
    removeComments: true,
  },
})
if (transpiled.diagnostics?.length) {
  const host = {
    getCanonicalFileName: file => file,
    getCurrentDirectory: () => root,
    getNewLine: () => '\n',
  }
  throw new Error(ts.formatDiagnostics(transpiled.diagnostics, host).trim())
}
if (/^\s*import[\s{]/m.test(transpiled.outputText) || /\brequire\s*\(/.test(transpiled.outputText)) {
  throw new Error('src/chat/kivioBlobShapes.ts must stay import-free; transpileModule output is imported from a data URL')
}

const encoded = Buffer.from(transpiled.outputText, 'utf8').toString('base64')
const geometry = await import(`data:text/javascript;charset=utf-8;base64,${encoded}`)

assert.equal(geometry.CX, 120)
assert.equal(geometry.CY, 120)
assert.equal(geometry.BODY_R, 78)
assert.equal(geometry.BODY_POINTS, BODY_LEN)
assert.equal(geometry.EYE_POINTS, EYE_LEN)
assert.deepEqual([...geometry.BODY_SHAPES], BODIES)
assert.deepEqual([...geometry.FACE_NAMES], FACES)

function pascal(name) {
  return name[0].toUpperCase() + name.slice(1)
}

function screaming(name) {
  return pascal(name).replace(/([a-z0-9])([A-Z])/g, '$1_$2').toUpperCase()
}

function rustFloat(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new Error(`non-finite geometry value: ${value}`)
  }
  if (Object.is(value, -0)) return '-0.0'
  const text = value.toString()
  const literal = /^-?\d+$/.test(text) ? `${text}.0` : text
  if (!/^-?(?:\d+\.\d+|\d+(?:\.\d+)?e[+-]?\d+)$/.test(literal)) {
    throw new Error(`cannot render ${value} as a Rust float (${text})`)
  }
  if (Number(literal) !== value) {
    throw new Error(`Rust literal ${literal} does not round-trip ${value}`)
  }
  return literal
}

function assertPolygon(label, points, count) {
  if (!Array.isArray(points) || points.length !== count) {
    throw new Error(`${label}: expected ${count} points, got ${points?.length}`)
  }
  for (const point of points) {
    if (!Array.isArray(point) || point.length !== 2) throw new Error(`${label}: bad point`)
    rustFloat(point[0])
    rustFloat(point[1])
  }
}

function assertBody(name, points) {
  assertPolygon(name, points, BODY_LEN)
  const minY = Math.min(...points.map(point => point[1]))
  if (Math.abs(points[0][1] - minY) > 1e-6) throw new Error(`${name}: first point is not the top`)
  const xs = points.map(point => point[0])
  const ys = points.map(point => point[1])
  if (!(Math.min(...xs) < geometry.CX && Math.max(...xs) > geometry.CX)) {
    throw new Error(`${name}: does not span the center x`)
  }
  if (!(Math.min(...ys) < geometry.CY && Math.max(...ys) > geometry.CY)) {
    throw new Error(`${name}: does not span the center y`)
  }
}

function assertFace(name, eyes) {
  if (!Array.isArray(eyes) || eyes.length !== 2) throw new Error(`${name}: expected two eyes`)
  eyes.forEach((eye, index) => assertPolygon(`${name} eye ${index}`, eye, EYE_LEN))
  const leftMax = Math.max(...eyes[0].map(point => point[0]))
  const rightMin = Math.min(...eyes[1].map(point => point[0]))
  if (!(leftMax < geometry.CX && rightMin > geometry.CX)) {
    throw new Error(`${name}: eyes are not split across the center line`)
  }
}

function formatPoint(point) {
  return `[${rustFloat(point[0])}, ${rustFloat(point[1])}]`
}

function formatPoly(points, indent) {
  const pad = ' '.repeat(indent)
  return points.map(point => `${pad}${formatPoint(point)},`).join('\n')
}

const bodies = BODIES.map(name => {
  const points = geometry.bodyPoints(name)
  assertBody(name, points)
  return [
    '#[rustfmt::skip]',
    `static BODY_${screaming(name)}: [[f64; 2]; ${BODY_LEN}] = [`,
    formatPoly(points, 4),
    '];',
  ].join('\n')
})

const blinks = []
const faces = FACES.map(name => {
  const spec = geometry.FACES[name]
  assert.equal(typeof spec, 'object', name)
  const blinksOn = geometry.faceBlinks(name)
  assert.equal(typeof blinksOn, 'boolean', name)
  assert.equal(blinksOn, !spec.noBlink, `${name} blink flag`)
  blinks.push([name, blinksOn])
  const eyes = geometry.facePoints(name)
  assertFace(name, eyes)
  const blocks = eyes.map(eye => `    [\n${formatPoly(eye, 8)}\n    ],`).join('\n')
  return [
    '#[rustfmt::skip]',
    `static FACE_${screaming(name)}: [[[f64; 2]; ${EYE_LEN}]; 2] = [`,
    blocks,
    '];',
  ].join('\n')
})

function matchArms(names, arm) {
  return names.map(arm).join('\n')
}

const bodyFn = [
  '/// Closed body outline in the source 240 viewBox. First point is the top; winding is clockwise.',
  `pub fn body_points(shape: BodyShape) -> &'static [[f64; 2]; ${BODY_LEN}] {`,
  '    match shape {',
  matchArms(BODIES, name => `        BodyShape::${pascal(name)} => &BODY_${screaming(name)},`),
  '    }',
  '}',
].join('\n')

const faceFn = [
  '/// Left eye, then right eye, placed with the source `placeEye` rules.',
  `pub fn face_points(face: FaceName) -> &'static [[[f64; 2]; ${EYE_LEN}]; 2] {`,
  '    match face {',
  matchArms(FACES, name => `        FaceName::${pascal(name)} => &FACE_${screaming(name)},`),
  '    }',
  '}',
].join('\n')

const blinkFn = [
  '/// Whether a blink scale may apply. `false` copies the TypeScript face `noBlink` flag.',
  'pub fn face_blinks(face: FaceName) -> bool {',
  '    match face {',
  matchArms(FACES, name => {
    const enabled = blinks.find(([key]) => key === name)[1]
    return `        FaceName::${pascal(name)} => ${enabled},`
  }),
  '    }',
  '}',
].join('\n')

const text = [
  '// Generated from src/chat/kivioBlobShapes.ts by scripts/generate-momo-native.mjs.',
  '// Included by shapes.rs. Do not edit by hand. Refresh with `npm run momo:generate`.',
  '//',
  '// Values are the JavaScript bodyPoints / facePoints / faceBlinks results in the',
  '// 240×240 viewBox. They are not scaled into the 128×128 pet.',
  '',
  bodyFn,
  '',
  faceFn,
  '',
  blinkFn,
  '',
  bodies.join('\n\n'),
  '',
  faces.join('\n\n'),
  '',
].join('\n')

if (!text.endsWith('\n')) throw new Error('generated geometry must end with a newline')

if (check) {
  let current
  try {
    current = readFileSync(outputPath, 'utf8')
  } catch {
    throw new Error(`${outputRel} is missing. Run npm run momo:generate.`)
  }
  if (current !== text) {
    const left = current.split('\n')
    const right = text.split('\n')
    let line = 1
    const limit = Math.max(left.length, right.length)
    while (line <= limit && left[line - 1] === right[line - 1]) line += 1
    throw new Error(`${outputRel} is out of date (first difference at line ${line}). Run npm run momo:generate.`)
  }
} else {
  writeFileSync(outputPath, text)
}

const noBlink = blinks.filter(([, enabled]) => !enabled).map(([name]) => name)
console.log(
  `${check ? 'Verified' : 'Generated'} ${outputRel}: ${BODIES.length} bodies, ${FACES.length} faces, noBlink=${noBlink.join(',')}.`,
)
