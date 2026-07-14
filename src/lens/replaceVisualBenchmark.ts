export type ReplaceVisualScene = 'ui' | 'document' | 'table' | 'photo' | 'unknown'

export type ReplaceVisualPlatform = {
  os: 'macos' | 'windows'
  scaleFactor: number
  display?: 'standard' | 'retina' | 'multi_display'
}

export type ReplaceVisualBounds = {
  x: number
  y: number
  width: number
  height: number
}

export type ReplaceVisualSlot = {
  id: string
  bounds: ReplaceVisualBounds
  anchor?: { x: number; y: number }
  column?: string
  rotationDeg?: number
}

export type ReplaceVisualFixtureCase = {
  version: 1
  id: string
  scene: ReplaceVisualScene
  required: boolean
  sourceImage: string
  protectionMask: string
  expectedGeometry: string
  translations: string
  platforms: ReplaceVisualPlatform[]
  tags: string[]
}

export type ReplaceVisualGeometry = {
  slots: ReplaceVisualSlot[]
}

export type ReplaceVisualTranslations = {
  translations: { id: string; text: string }[]
}

export type ReplaceGeometryMetrics = {
  matchedSlots: number
  missingSlotIds: string[]
  unexpectedSlotIds: string[]
  meanTopError: number
  maxTopError: number
  meanLeftError: number
  maxLeftError: number
  maxFirstAnchorDrift: number
  minIntersectionOverUnion: number
  maxForbiddenOverlapRatio: number
  crossColumnOverlapCount: number
}

export type ReplacePixelMetrics = {
  protectedPixelCount: number
  protectedPixelChangeRatio: number
  outsideEraseMaskPixelCount: number
  outsideEraseMaskChangeRatio: number
}

export type ReplaceTranslationMetrics = {
  expectedCount: number
  completeCount: number
  completenessRatio: number
  missingIds: string[]
  mismatchedIds: string[]
  duplicateIds: string[]
  unexpectedIds: string[]
}

export type ReplaceVisualFixtureMetrics = {
  geometry: ReplaceGeometryMetrics
  pixels: ReplacePixelMetrics
  translation: ReplaceTranslationMetrics
  hotPathMs?: number
}

export type ReplaceVisualThresholds = {
  maxTopError: number
  maxLeftError: number
  maxFirstAnchorDrift: number
  minIntersectionOverUnion: number
  maxForbiddenOverlapRatio: number
  maxCrossColumnOverlapCount: number
  maxProtectedPixelChangeRatio: number
  maxOutsideEraseMaskChangeRatio: number
  minTranslationCompleteness: number
  maxHotPathMs: number
}

export type ReplaceVisualFixtureResult = {
  fixtureId: string
  scene: ReplaceVisualScene
  required: boolean
  passed: boolean
  failures: string[]
  metrics: ReplaceVisualFixtureMetrics
}

export type ReplaceVisualBucketResult = {
  scene: ReplaceVisualScene
  passed: boolean
  fixtureCount: number
  failedFixtureIds: string[]
}

export const REPLACE_VISUAL_THRESHOLDS: Record<ReplaceVisualScene, ReplaceVisualThresholds> = {
  ui: threshold({ maxTopError: 3, maxLeftError: 3, maxFirstAnchorDrift: 3, minIntersectionOverUnion: 0.82, maxHotPathMs: 1_000 }),
  document: threshold({ maxTopError: 4, maxLeftError: 4, maxFirstAnchorDrift: 3, minIntersectionOverUnion: 0.78, maxHotPathMs: 1_000 }),
  table: threshold({ maxTopError: 3, maxLeftError: 3, maxFirstAnchorDrift: 3, minIntersectionOverUnion: 0.85, maxHotPathMs: 1_000 }),
  photo: threshold({ maxTopError: 6, maxLeftError: 6, maxFirstAnchorDrift: 6, minIntersectionOverUnion: 0.65, maxHotPathMs: 3_000 }),
  unknown: threshold({ maxTopError: 3, maxLeftError: 3, maxFirstAnchorDrift: 3, minIntersectionOverUnion: 0.82, maxHotPathMs: 1_000 }),
}

function threshold(overrides: Partial<ReplaceVisualThresholds>): ReplaceVisualThresholds {
  return {
    maxTopError: 3,
    maxLeftError: 3,
    maxFirstAnchorDrift: 3,
    minIntersectionOverUnion: 0.82,
    maxForbiddenOverlapRatio: 0.01,
    maxCrossColumnOverlapCount: 0,
    maxProtectedPixelChangeRatio: 0,
    maxOutsideEraseMaskChangeRatio: 0,
    minTranslationCompleteness: 1,
    maxHotPathMs: 1_000,
    ...overrides,
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function requireString(record: Record<string, unknown>, key: string): string {
  const value = record[key]
  if (typeof value !== 'string' || !value.trim()) throw new Error(`fixture.${key} must be a non-empty string`)
  return value
}

export function parseReplaceVisualFixtureCase(value: unknown): ReplaceVisualFixtureCase {
  if (!isRecord(value)) throw new Error('fixture must be an object')
  if (value.version !== 1) throw new Error('fixture.version must be 1')
  if (!['ui', 'document', 'table', 'photo', 'unknown'].includes(String(value.scene))) {
    throw new Error('fixture.scene is invalid')
  }
  if (typeof value.required !== 'boolean') throw new Error('fixture.required must be boolean')
  if (!Array.isArray(value.platforms) || value.platforms.length === 0) {
    throw new Error('fixture.platforms must not be empty')
  }
  const platforms = value.platforms.map((item, index) => {
    if (!isRecord(item) || !['macos', 'windows'].includes(String(item.os))) {
      throw new Error(`fixture.platforms[${index}].os is invalid`)
    }
    if (typeof item.scaleFactor !== 'number' || item.scaleFactor <= 0) {
      throw new Error(`fixture.platforms[${index}].scaleFactor must be positive`)
    }
    return {
      os: item.os as ReplaceVisualPlatform['os'],
      scaleFactor: item.scaleFactor,
      display: item.display as ReplaceVisualPlatform['display'],
    }
  })
  if (!Array.isArray(value.tags) || value.tags.some(tag => typeof tag !== 'string')) {
    throw new Error('fixture.tags must be strings')
  }
  return {
    version: 1,
    id: requireString(value, 'id'),
    scene: value.scene as ReplaceVisualScene,
    required: value.required,
    sourceImage: requireString(value, 'sourceImage'),
    protectionMask: requireString(value, 'protectionMask'),
    expectedGeometry: requireString(value, 'expectedGeometry'),
    translations: requireString(value, 'translations'),
    platforms,
    tags: value.tags as string[],
  }
}

function area(bounds: ReplaceVisualBounds): number {
  return Math.max(0, bounds.width) * Math.max(0, bounds.height)
}

function intersectionArea(a: ReplaceVisualBounds, b: ReplaceVisualBounds): number {
  const width = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x))
  const height = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y))
  return width * height
}

function intersectionOverUnion(a: ReplaceVisualBounds, b: ReplaceVisualBounds): number {
  const intersection = intersectionArea(a, b)
  const union = area(a) + area(b) - intersection
  return union <= 0 ? 0 : intersection / union
}

function mean(values: number[]): number {
  return values.length === 0 ? 0 : values.reduce((sum, value) => sum + value, 0) / values.length
}

export function computeReplaceGeometryMetrics(
  expected: ReplaceVisualSlot[],
  actual: ReplaceVisualSlot[],
): ReplaceGeometryMetrics {
  const expectedById = new Map(expected.map(slot => [slot.id, slot]))
  const actualById = new Map(actual.map(slot => [slot.id, slot]))
  const matched = expected.flatMap(slot => {
    const observed = actualById.get(slot.id)
    return observed ? [{ expected: slot, actual: observed }] : []
  })
  const topErrors = matched.map(pair => Math.abs(pair.expected.bounds.y - pair.actual.bounds.y))
  const leftErrors = matched.map(pair => Math.abs(pair.expected.bounds.x - pair.actual.bounds.x))
  const anchorDrifts = matched.map(pair => {
    const expectedAnchor = pair.expected.anchor ?? { x: pair.expected.bounds.x, y: pair.expected.bounds.y }
    const actualAnchor = pair.actual.anchor ?? { x: pair.actual.bounds.x, y: pair.actual.bounds.y }
    return Math.hypot(expectedAnchor.x - actualAnchor.x, expectedAnchor.y - actualAnchor.y)
  })
  const intersectionOverUnions = matched.map(pair => intersectionOverUnion(pair.expected.bounds, pair.actual.bounds))

  let maxForbiddenOverlapRatio = 0
  let crossColumnOverlapCount = 0
  for (let left = 0; left < expected.length; left += 1) {
    for (let right = left + 1; right < expected.length; right += 1) {
      if (intersectionArea(expected[left].bounds, expected[right].bounds) > 0) continue
      const actualLeft = actualById.get(expected[left].id)
      const actualRight = actualById.get(expected[right].id)
      if (!actualLeft || !actualRight) continue
      const overlap = intersectionArea(actualLeft.bounds, actualRight.bounds)
      const ratio = overlap / Math.max(1, Math.min(area(actualLeft.bounds), area(actualRight.bounds)))
      maxForbiddenOverlapRatio = Math.max(maxForbiddenOverlapRatio, ratio)
      if (overlap > 0 && expected[left].column && expected[right].column && expected[left].column !== expected[right].column) {
        crossColumnOverlapCount += 1
      }
    }
  }

  return {
    matchedSlots: matched.length,
    missingSlotIds: expected.filter(slot => !actualById.has(slot.id)).map(slot => slot.id),
    unexpectedSlotIds: actual.filter(slot => !expectedById.has(slot.id)).map(slot => slot.id),
    meanTopError: mean(topErrors),
    maxTopError: Math.max(0, ...topErrors),
    meanLeftError: mean(leftErrors),
    maxLeftError: Math.max(0, ...leftErrors),
    maxFirstAnchorDrift: Math.max(0, ...anchorDrifts),
    minIntersectionOverUnion: intersectionOverUnions.length === 0 ? 0 : Math.min(...intersectionOverUnions),
    maxForbiddenOverlapRatio,
    crossColumnOverlapCount,
  }
}

function pixelChanged(source: Uint8Array, output: Uint8Array, pixelIndex: number): boolean {
  const offset = pixelIndex * 4
  return source[offset] !== output[offset]
    || source[offset + 1] !== output[offset + 1]
    || source[offset + 2] !== output[offset + 2]
    || source[offset + 3] !== output[offset + 3]
}

export function computeReplacePixelMetrics(
  sourceRgba: Uint8Array,
  outputRgba: Uint8Array,
  protectionMask: Uint8Array,
  eraseMask: Uint8Array,
): ReplacePixelMetrics {
  if (sourceRgba.length !== outputRgba.length || sourceRgba.length % 4 !== 0) {
    throw new Error('source and output RGBA buffers must have equal lengths')
  }
  const pixelCount = sourceRgba.length / 4
  if (protectionMask.length !== pixelCount || eraseMask.length !== pixelCount) {
    throw new Error('pixel masks must match the RGBA pixel count')
  }
  let protectedPixelCount = 0
  let changedProtectedPixels = 0
  let outsideEraseMaskPixelCount = 0
  let changedOutsideEraseMaskPixels = 0
  for (let index = 0; index < pixelCount; index += 1) {
    const changed = pixelChanged(sourceRgba, outputRgba, index)
    if (protectionMask[index] > 0) {
      protectedPixelCount += 1
      if (changed) changedProtectedPixels += 1
    }
    if (eraseMask[index] === 0) {
      outsideEraseMaskPixelCount += 1
      if (changed) changedOutsideEraseMaskPixels += 1
    }
  }
  return {
    protectedPixelCount,
    protectedPixelChangeRatio: changedProtectedPixels / Math.max(1, protectedPixelCount),
    outsideEraseMaskPixelCount,
    outsideEraseMaskChangeRatio: changedOutsideEraseMaskPixels / Math.max(1, outsideEraseMaskPixelCount),
  }
}

export function computeReplaceTranslationMetrics(
  expected: ReplaceVisualTranslations['translations'],
  actual: ReplaceVisualTranslations['translations'],
): ReplaceTranslationMetrics {
  const expectedById = new Map(expected.map(item => [item.id, item.text.trim()]))
  const actualById = new Map<string, string>()
  const duplicateIds = new Set<string>()
  for (const item of actual) {
    if (actualById.has(item.id)) duplicateIds.add(item.id)
    else actualById.set(item.id, item.text.trim())
  }
  const missingIds: string[] = []
  const mismatchedIds: string[] = []
  let completeCount = 0
  for (const [id, text] of expectedById) {
    const observed = actualById.get(id)
    if (observed === undefined || observed.length === 0) missingIds.push(id)
    else if (observed !== text || duplicateIds.has(id)) mismatchedIds.push(id)
    else completeCount += 1
  }
  return {
    expectedCount: expected.length,
    completeCount,
    completenessRatio: completeCount / Math.max(1, expected.length),
    missingIds,
    mismatchedIds,
    duplicateIds: [...duplicateIds].sort(),
    unexpectedIds: [...actualById.keys()].filter(id => !expectedById.has(id)).sort(),
  }
}

export function evaluateReplaceVisualFixture(
  fixtureId: string,
  scene: ReplaceVisualScene,
  required: boolean,
  metrics: ReplaceVisualFixtureMetrics,
  thresholds = REPLACE_VISUAL_THRESHOLDS[scene],
): ReplaceVisualFixtureResult {
  const failures: string[] = []
  const { geometry, pixels, translation } = metrics
  if (geometry.missingSlotIds.length > 0) failures.push(`missing slots: ${geometry.missingSlotIds.join(', ')}`)
  if (geometry.unexpectedSlotIds.length > 0) failures.push(`unexpected slots: ${geometry.unexpectedSlotIds.join(', ')}`)
  if (geometry.maxTopError > thresholds.maxTopError) failures.push(`top error ${geometry.maxTopError.toFixed(2)}px`)
  if (geometry.maxLeftError > thresholds.maxLeftError) failures.push(`left error ${geometry.maxLeftError.toFixed(2)}px`)
  if (geometry.maxFirstAnchorDrift > thresholds.maxFirstAnchorDrift) failures.push(`anchor drift ${geometry.maxFirstAnchorDrift.toFixed(2)}px`)
  if (geometry.minIntersectionOverUnion < thresholds.minIntersectionOverUnion) failures.push(`minimum IoU ${geometry.minIntersectionOverUnion.toFixed(3)}`)
  if (geometry.maxForbiddenOverlapRatio > thresholds.maxForbiddenOverlapRatio) failures.push(`forbidden overlap ${geometry.maxForbiddenOverlapRatio.toFixed(3)}`)
  if (geometry.crossColumnOverlapCount > thresholds.maxCrossColumnOverlapCount) failures.push(`cross-column overlaps ${geometry.crossColumnOverlapCount}`)
  if (pixels.protectedPixelChangeRatio > thresholds.maxProtectedPixelChangeRatio) failures.push(`protected pixels changed ${(pixels.protectedPixelChangeRatio * 100).toFixed(3)}%`)
  if (pixels.outsideEraseMaskChangeRatio > thresholds.maxOutsideEraseMaskChangeRatio) failures.push(`pixels outside mask changed ${(pixels.outsideEraseMaskChangeRatio * 100).toFixed(3)}%`)
  if (translation.completenessRatio < thresholds.minTranslationCompleteness) failures.push(`translation completeness ${(translation.completenessRatio * 100).toFixed(1)}%`)
  if (metrics.hotPathMs !== undefined && metrics.hotPathMs > thresholds.maxHotPathMs) failures.push(`hot path ${metrics.hotPathMs.toFixed(1)}ms`)
  return { fixtureId, scene, required, passed: failures.length === 0, failures, metrics }
}

export function aggregateReplaceVisualGate(
  results: ReplaceVisualFixtureResult[],
  requiredScenes: ReplaceVisualScene[],
): { passed: boolean; buckets: ReplaceVisualBucketResult[] } {
  const buckets = requiredScenes.map(scene => {
    const fixtures = results.filter(result => result.scene === scene && result.required)
    const failedFixtureIds = fixtures.filter(result => !result.passed).map(result => result.fixtureId)
    return {
      scene,
      fixtureCount: fixtures.length,
      failedFixtureIds,
      passed: fixtures.length > 0 && failedFixtureIds.length === 0,
    }
  })
  return { passed: buckets.every(bucket => bucket.passed), buckets }
}
