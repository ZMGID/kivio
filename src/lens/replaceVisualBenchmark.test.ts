import { describe, expect, it } from 'vitest'
import menuCaseJson from '../../tests/fixtures/replace-translation/v1/menu-list/case.json'
import menuGeometry from '../../tests/fixtures/replace-translation/v1/menu-list/expected_geometry.json'
import menuTranslations from '../../tests/fixtures/replace-translation/v1/menu-list/translations.json'
import documentCaseJson from '../../tests/fixtures/replace-translation/v1/document-paragraph/case.json'
import tableCaseJson from '../../tests/fixtures/replace-translation/v1/table-grid/case.json'
import codeBadgeCaseJson from '../../tests/fixtures/replace-translation/v1/code-badge/case.json'
import photoCaseJson from '../../tests/fixtures/replace-translation/v1/photo-gradient/case.json'
import {
  aggregateReplaceVisualGate,
  computeReplaceGeometryMetrics,
  computeReplacePixelMetrics,
  computeReplaceTranslationMetrics,
  evaluateReplaceVisualFixture,
  parseReplaceVisualFixtureCase,
  type ReplaceVisualFixtureMetrics,
  type ReplaceVisualFixtureResult,
  type ReplaceVisualGeometry,
  type ReplaceVisualScene,
  type ReplaceVisualTranslations,
} from './replaceVisualBenchmark'

const fixtureCases = [menuCaseJson, documentCaseJson, tableCaseJson, codeBadgeCaseJson, photoCaseJson]

function passingMetrics(): ReplaceVisualFixtureMetrics {
  return {
    geometry: {
      matchedSlots: 1,
      missingSlotIds: [],
      unexpectedSlotIds: [],
      meanTopError: 0,
      maxTopError: 0,
      meanLeftError: 0,
      maxLeftError: 0,
      maxFirstAnchorDrift: 0,
      minIntersectionOverUnion: 1,
      maxForbiddenOverlapRatio: 0,
      crossColumnOverlapCount: 0,
    },
    pixels: {
      protectedPixelCount: 10,
      protectedPixelChangeRatio: 0,
      outsideEraseMaskPixelCount: 10,
      outsideEraseMaskChangeRatio: 0,
    },
    translation: {
      expectedCount: 1,
      completeCount: 1,
      completenessRatio: 1,
      missingIds: [],
      mismatchedIds: [],
      duplicateIds: [],
      unexpectedIds: [],
    },
    hotPathMs: 100,
  }
}

function result(scene: ReplaceVisualScene, passed: boolean): ReplaceVisualFixtureResult {
  const metrics = passingMetrics()
  if (!passed) metrics.geometry.maxFirstAnchorDrift = 40
  return evaluateReplaceVisualFixture(`${scene}-fixture`, scene, true, metrics)
}

describe('replace visual fixture schema', () => {
  it('keeps a versioned cross-platform fixture matrix for all first-release scenes', () => {
    const parsed = fixtureCases.map(parseReplaceVisualFixtureCase)
    expect(new Set(parsed.map(fixture => fixture.scene))).toEqual(new Set(['ui', 'document', 'table', 'photo']))
    for (const fixture of parsed) {
      expect(fixture.platforms.some(platform => platform.os === 'macos')).toBe(true)
      expect(fixture.platforms.some(platform => platform.os === 'windows')).toBe(true)
      expect(fixture.sourceImage).toBe('source.png')
      expect(fixture.protectionMask).toBe('expected_protection.png')
    }
  })

  it('rejects an unversioned or platform-free fixture', () => {
    expect(() => parseReplaceVisualFixtureCase({ ...menuCaseJson, version: 2 })).toThrow('version')
    expect(() => parseReplaceVisualFixtureCase({ ...menuCaseJson, platforms: [] })).toThrow('platforms')
  })
})

describe('replace visual geometry metrics', () => {
  const expected = (menuGeometry as ReplaceVisualGeometry).slots

  it('detects the historical whole-menu upward shift', () => {
    const shifted = expected.map(slot => ({
      ...slot,
      bounds: { ...slot.bounds, y: slot.bounds.y - 36 },
      anchor: slot.anchor ? { ...slot.anchor, y: slot.anchor.y - 36 } : undefined,
    }))
    const metrics = computeReplaceGeometryMetrics(expected, shifted)
    expect(metrics.maxTopError).toBe(36)
    expect(metrics.maxFirstAnchorDrift).toBe(36)
    expect(evaluateReplaceVisualFixture('menu-shift', 'ui', true, {
      ...passingMetrics(),
      geometry: metrics,
    }).passed).toBe(false)
  })

  it('detects table slots crossing a hard column boundary', () => {
    const tableExpected = [
      { id: 'left', bounds: { x: 10, y: 10, width: 80, height: 30 }, column: 'left' },
      { id: 'right', bounds: { x: 110, y: 10, width: 80, height: 30 }, column: 'right' },
    ]
    const observed = [
      tableExpected[0],
      { ...tableExpected[1], bounds: { x: 70, y: 10, width: 120, height: 30 } },
    ]
    const metrics = computeReplaceGeometryMetrics(tableExpected, observed)
    expect(metrics.crossColumnOverlapCount).toBe(1)
    expect(metrics.maxForbiddenOverlapRatio).toBeGreaterThan(0)
  })
})

describe('replace visual pixel and translation metrics', () => {
  it('detects icon or table-line changes even when the changed area is tiny', () => {
    const source = new Uint8Array(4 * 4)
    source.fill(255)
    const output = source.slice()
    output[0] = 0
    const protection = new Uint8Array([255, 0, 0, 0])
    const eraseMask = new Uint8Array([0, 255, 0, 0])
    const metrics = computeReplacePixelMetrics(source, output, protection, eraseMask)
    expect(metrics.protectedPixelChangeRatio).toBe(1)
    expect(metrics.outsideEraseMaskChangeRatio).toBeGreaterThan(0)
  })

  it('requires exact complete translations and flags duplicates independently', () => {
    const expected = (menuTranslations as ReplaceVisualTranslations).translations
    const actual = [
      { id: expected[0].id, text: expected[0].text },
      { id: expected[0].id, text: expected[0].text },
      { id: expected[1].id, text: '' },
    ]
    const metrics = computeReplaceTranslationMetrics(expected, actual)
    expect(metrics.completenessRatio).toBe(0)
    expect(metrics.duplicateIds).toEqual([expected[0].id])
    expect(metrics.missingIds).toContain(expected[1].id)
  })
})

describe('replace visual release gate', () => {
  it('fails by scene bucket instead of hiding a broken photo path in an average', () => {
    const report = aggregateReplaceVisualGate([
      result('ui', true),
      result('document', true),
      result('table', true),
      result('photo', false),
    ], ['ui', 'document', 'table', 'photo'])
    expect(report.passed).toBe(false)
    expect(report.buckets.find(bucket => bucket.scene === 'photo')?.passed).toBe(false)
    expect(report.buckets.filter(bucket => bucket.scene !== 'photo').every(bucket => bucket.passed)).toBe(true)
  })

  it('fails when a required platform scene has no fixture at all', () => {
    const report = aggregateReplaceVisualGate([result('ui', true)], ['ui', 'table'])
    expect(report.passed).toBe(false)
    expect(report.buckets.find(bucket => bucket.scene === 'table')?.fixtureCount).toBe(0)
  })
})
